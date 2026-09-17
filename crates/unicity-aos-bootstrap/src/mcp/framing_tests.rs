use tokio::io::{AsyncWriteExt as _, BufReader};

use super::read_frame;

#[tokio::test]
async fn interrupted_read_preserves_partial_frame() {
    let (mut writer, reader) = tokio::io::duplex(128);
    let mut reader = BufReader::new(reader);
    let mut frame = Vec::new();
    writer.write_all(b"first ").await.unwrap();
    // Poll until the available prefix has been consumed and the read waits for
    // its delimiter, then model the other branch winning the serve select.
    {
        let read = read_frame(&mut reader, &mut frame);
        tokio::pin!(read);
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(read.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
    }
    assert_eq!(frame, b"first ");
    writer.write_all(b"second\n").await.unwrap();
    let size = read_frame(&mut reader, &mut frame).await.unwrap();
    assert_eq!(frame, b"first second\n");
    assert_eq!(size, frame.len());
}

#[tokio::test]
async fn retained_prefix_at_eof_is_not_an_empty_stream() {
    let (writer, reader) = tokio::io::duplex(128);
    let mut reader = BufReader::new(reader);
    let mut frame = b"retained prefix".to_vec();
    drop(writer);
    assert_eq!(read_frame(&mut reader, &mut frame).await.unwrap(), 15);
    assert_eq!(frame, b"retained prefix");
    frame.clear();
    assert_eq!(read_frame(&mut reader, &mut frame).await.unwrap(), 0);
}
