//! The gzip decode of a response body, piece by piece (plan section 8.8).

use std::io::{self, Write as _};

use bytes::Bytes;
use flate2::write::GzDecoder;

/// Decodes the pieces of one gzip body in their sequence.
///
/// The output of one piece is in memory at one time. A piece of an HTTP/2 body has 16 KiB
/// or less, and gzip expands 1,000 times at most, so this buffer has a limit.
pub(super) struct Gunzip {
    decoder: GzDecoder<Vec<u8>>,
}

impl Gunzip {
    pub(super) fn new() -> Self {
        Self { decoder: GzDecoder::new(Vec::new()) }
    }

    /// Decodes one piece. The result can be empty: the decoder then needs more data.
    pub(super) fn feed(&mut self, piece: &[u8]) -> io::Result<Bytes> {
        self.decoder.write_all(piece)?;
        Ok(self.take())
    }

    /// The end of the body. An error means that the gzip data is not complete, so a list
    /// that the connection cut does not look like a short list.
    pub(super) fn finish(&mut self) -> io::Result<Bytes> {
        self.decoder.try_finish()?;
        Ok(self.take())
    }

    fn take(&mut self) -> Bytes {
        Bytes::from(std::mem::take(self.decoder.get_mut()))
    }
}

#[cfg(test)]
pub(super) mod tests {
    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;

    pub(in crate::body) fn gzip(text: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(text).expect("a write to memory");
        encoder.finish().expect("a write to memory")
    }

    fn list() -> Vec<u8> {
        (0..2000).flat_map(|item| format!(r#"{{"metadata":{{"name":"pod-{item}"}}}},"#).into_bytes()).collect()
    }

    #[test]
    fn each_piece_size_gives_the_same_text() {
        let (text, packed) = (list(), gzip(&list()));
        assert!(packed.len() * 5 < text.len(), "the test data must compress: {} to {}", text.len(), packed.len());
        for size in [1, 7, 1000, packed.len()] {
            let mut gunzip = Gunzip::new();
            let mut decoded = Vec::new();
            for piece in packed.chunks(size) {
                decoded.extend_from_slice(&gunzip.feed(piece).expect("good data"));
            }
            decoded.extend_from_slice(&gunzip.finish().expect("complete data"));
            assert!(decoded == text, "pieces of {size} bytes");
        }
    }

    #[test]
    fn data_that_ends_early_or_is_not_gzip_is_an_error() {
        let packed = gzip(&list());
        let mut cut = Gunzip::new();
        cut.feed(&packed[..packed.len() - 9]).expect("good data to this point");
        assert!(cut.finish().is_err(), "the end of the gzip data is absent");

        let mut plain = Gunzip::new();
        assert!(plain.feed(br#"{"kind":"PodList"}"#).is_err());
    }
}
