//! Tiny, generated-at-runtime bridge to the reviewed Station v2 publisher.
//!
//! This source intentionally lives in the AOS adapter rather than in the
//! Station repository.  The adapter exports and freezes the exact reviewed Git
//! objects (commit `fd233f91b40d00c5f721ba99436ce30eca9036ba`, tree
//! `0bf775f225bfb9177d5788ec8c32d5453f75c660`), so the call below cannot
//! silently resolve a different tool version or live checkout path.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use astrid_station_protocol::{
    ActorId, BuildProvenance, CanonicalSemVer, Coordinate, Digest, GitObjectId, MirrorUrl,
    PublicationRecordV2, PublisherIdentity, SourceProvenance, StationId,
};
use astrid_station_publish::{prepare_v2, EmptyPreflightV2, PublishOptions};

fn argument(name: &str) -> String {
    let mut args = env::args().skip(1);
    while let Some(value) = args.next() {
        if value == name {
            return args
                .next()
                .unwrap_or_else(|| panic!("missing value for {name}"));
        }
    }
    panic!("missing required argument {name}");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let artifact = PathBuf::from(argument("--artifact"));
    let output = PathBuf::from(argument("--output"));
    let coordinate = Coordinate::from_str(&argument("--coordinate"))?;
    let version = CanonicalSemVer::parse(&argument("--version"))?;
    let station_id = StationId::new(argument("--station-id"))?;
    let station_base = MirrorUrl::new(argument("--station-base"))?;
    let source_repository = MirrorUrl::new(argument("--source-repository"))?;
    let source_commit = GitObjectId::new(argument("--source-commit"))?;
    let source_tree = GitObjectId::new(argument("--source-tree"))?;
    let source_tag = argument("--source-tag");
    let source_digest = Digest::parse(&argument("--source-digest"))?;
    let statement_digest = Digest::parse(&argument("--statement-digest"))?;
    let publisher_digest = Digest::parse(&argument("--publisher-digest"))?;
    let publisher = PublisherIdentity::new(
        ActorId::new(argument("--publisher"))?,
        publisher_digest,
    );
    let source = SourceProvenance::new(
        source_repository,
        argument("--github-owner-id").parse()?,
        argument("--github-repository-id").parse()?,
        source_commit,
        source_tree,
        source_tag,
        None,
        source_digest,
    )?;
    let provenance = BuildProvenance::new(
        argument("--predicate-type"),
        statement_digest,
        MirrorUrl::new(argument("--builder-identity"))?,
        argument("--attestation-identity"),
    )?;
    let options = PublishOptions::new(
        &artifact,
        station_id,
        station_base,
        coordinate,
        version,
        Vec::new(),
        publisher,
        source,
        provenance,
        argument("--runtime"),
        argument("--abi"),
        &output,
    );
    let prepared = prepare_v2(&options, &EmptyPreflightV2)?;
    let record_path = prepared.output_path().to_path_buf();
    let artifact_path = prepared.artifact_output_path().to_path_buf();
    let record_bytes = prepared.json_bytes()?;
    let decoded: PublicationRecordV2 = serde_json::from_slice(&record_bytes)?;
    if decoded.publication_digest() != prepared.record().publication_digest() {
        return Err("serialized PublicationRecordV2 did not round-trip".into());
    }
    write_regular(&record_path, &record_bytes)?;
    write_regular(&artifact_path, prepared.artifact_bytes())?;
    let result = serde_json::json!({
        "schema": "aos-station-prepare-v2-result-v1",
        "record": record_path.strip_prefix(&output)?.to_string_lossy(),
        "artifact": artifact_path.strip_prefix(&output)?.to_string_lossy(),
        "coordinate": prepared.record().coordinate().to_string(),
        "version": prepared.record().version(),
        "publication_digest": prepared.record().publication_digest(),
        "package_digest": prepared.record().package().embedded_identity.package_digest(),
        "manifest_digest": prepared.record().package().manifest_digest,
        "content_digest": prepared.record().package().capsule_content_digest,
        "artifact_size": prepared.record().artifact().size,
        "classification": format!("{:?}", prepared.classification()),
        "readiness": false,
        "fixture": true,
    });
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn write_regular(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.is_symlink() {
        return Err(format!("refusing to overwrite symlink {}", path.display()).into());
    }
    fs::write(path, bytes)?;
    Ok(())
}
