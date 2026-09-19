//! Authenticated private-input transport shared with the native app's wire contract.
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;
use std::time::{Duration, Instant};

use astrid_crypto::KeyPair;
use serde::Deserialize;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::Event;
use super::model::{Destination, Kind, Request, display};
use super::transport::{private_read, same_user};

const MAX_FRAME: usize = 2 * 1024 * 1024;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Configuration {
    socket_path: PathBuf,
    principal: String,
    private_key_path: PathBuf,
    token_path: PathBuf,
    input_timeout_seconds: f64,
    io_timeout_seconds: f64,
}

fn invalid() -> io::Error {
    io::Error::other("private input connection could not be verified")
}

fn write_frame(stream: &mut UnixStream, value: &Value) -> io::Result<()> {
    let body = Zeroizing::new(serde_json::to_vec(value)?);
    stream.write_all(&(body.len() as u32).to_be_bytes())?;
    stream.write_all(&body)
}

fn read_frame(stream: &mut UnixStream) -> io::Result<Value> {
    let mut size = [0; 4];
    stream.read_exact(&mut size)?;
    let size = u32::from_be_bytes(size) as usize;
    if size == 0 || size > MAX_FRAME {
        return Err(invalid());
    }
    let mut bytes = Zeroizing::new(vec![0; size]);
    stream.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}

pub(super) fn connect(
    path: &Path,
    events: SyncSender<Event>,
    generation: u64,
) -> io::Result<UnixStream> {
    let config: Configuration =
        serde_json::from_slice(&private_read(path, 16_384)?).map_err(|_| invalid())?;
    if config.principal == "anonymous"
        || config.principal.is_empty()
        || config.principal.len() > 128
        || !config
            .principal
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        || !config.io_timeout_seconds.is_finite()
        || !(0.0..=60.0).contains(&config.io_timeout_seconds)
        || config.io_timeout_seconds == 0.0
        || !config.input_timeout_seconds.is_finite()
        || !(1.0..=3600.0).contains(&config.input_timeout_seconds)
    {
        return Err(invalid());
    }
    let secret = Zeroizing::new(private_read(&config.private_key_path, 32)?);
    let key = KeyPair::from_secret_key(&secret).map_err(|_| invalid())?;
    let token = Zeroizing::new(private_read(&config.token_path, 64)?);
    if token.len() != 64
        || !token
            .iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
    {
        return Err(invalid());
    }
    let mut stream = UnixStream::connect(&config.socket_path)?;
    same_user(&stream)?;
    stream.set_read_timeout(Some(Duration::from_secs_f64(config.io_timeout_seconds)))?;
    stream.set_write_timeout(Some(Duration::from_secs_f64(config.io_timeout_seconds)))?;
    let mut auth = json!({"token":std::str::from_utf8(&token).map_err(|_| invalid())?,"protocol_version":1,
        "client_version":"aos-console","claimed_principal":config.principal});
    write_frame(&mut stream, &auth)?;
    let challenge = read_frame(&mut stream)?;
    let nonce = challenge["challenge"].as_str().ok_or_else(invalid)?;
    if challenge["status"] != "ok"
        || challenge["protocol_version"] != 1
        || nonce.len() != 64
        || !nonce
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(invalid());
    }
    auth["signature"] = json!(
        key.sign(format!("astrid-principal-auth:v1:{}:{nonce}", config.principal).as_bytes())
            .to_hex()
    );
    write_frame(&mut stream, &auth)?;
    let authenticated = read_frame(&mut stream)?;
    if authenticated["status"] != "ok"
        || authenticated["protocol_version"] != 1
        || !authenticated["challenge"].is_null()
    {
        return Err(invalid());
    }
    stream.set_read_timeout(None)?;
    let control = stream.try_clone()?;
    std::thread::spawn(move || {
        while let Ok(frame) = read_frame(&mut stream) {
            let event = match frame["topic"].as_str() {
                Some("astrid.v1.private.elicit.request") => match request(
                    frame,
                    &config.principal,
                    &stream,
                    generation,
                    config.input_timeout_seconds,
                ) {
                    Ok(request) => Event::Request(request),
                    Err(_) => break,
                },
                Some("astrid.v1.private.elicit.result") => {
                    if frame["principal"] != config.principal {
                        break;
                    }
                    let Some(id) = frame["payload"]["request_id"].as_str() else {
                        break;
                    };
                    Event::Delivered {
                        id: id.into(),
                        delivered: frame["payload"]["status"] == "delivered",
                    }
                }
                _ => continue,
            };
            if events.try_send(event).is_err() {
                break;
            }
        }
        let _ = stream.shutdown(Shutdown::Both);
        let _ = events.try_send(Event::Disconnected(generation));
    });
    Ok(control)
}

fn request(
    frame: Value,
    principal: &str,
    stream: &UnixStream,
    generation: u64,
    timeout: f64,
) -> io::Result<Request> {
    if frame["principal"] != principal
        || frame["source_id"] != "00000000-0000-0000-0000-000000000000"
    {
        return Err(invalid());
    }
    let payload = &frame["payload"];
    let id = payload["request_id"].as_str().ok_or_else(invalid)?;
    uuid::Uuid::parse_str(id).map_err(|_| invalid())?;
    let field = &payload["field"];
    let capsule = payload["capsule_id"].as_str().ok_or_else(invalid)?;
    let prompt = field["prompt"].as_str().ok_or_else(invalid)?;
    let key = field["key"].as_str().ok_or_else(invalid)?;
    if [capsule, key]
        .iter()
        .any(|s| s.is_empty() || s.len() > 128 || s.chars().any(char::is_control))
        || prompt.is_empty()
        || prompt.len() > 8192
    {
        return Err(invalid());
    }
    let kind = match field["field_type"].as_str() {
        Some("Text") => Kind::Text,
        Some("Secret") => Kind::Secret,
        Some("Array") => Kind::Array,
        Some(_) => return Err(invalid()),
        None => {
            let options: Vec<String> = serde_json::from_value(field["field_type"]["Enum"].clone())
                .map_err(|_| invalid())?;
            if options.is_empty()
                || options.len() > 64
                || options
                    .iter()
                    .any(|o| o.is_empty() || o.len() > 256 || o.chars().any(char::is_control))
            {
                return Err(invalid());
            }
            Kind::Select(options)
        }
    };
    if matches!(kind, Kind::Secret | Kind::Array) && !field["default"].is_null() {
        return Err(invalid());
    }
    Ok(Request {
        id: id.into(),
        title: display(capsule),
        principal: principal.into(),
        message: display(prompt),
        detail: format!(
            "Private input · {}\nSent directly to your runtime. Never sent through the agent.",
            display(key)
        ),
        kind,
        deadline: Instant::now() + Duration::from_secs_f64(timeout),
        destination: Destination::Private {
            stream: stream.try_clone()?,
            generation,
        },
    })
}
