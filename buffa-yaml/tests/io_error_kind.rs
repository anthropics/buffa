use std::io::{self, Read, Write};

use buffa_types::{google::protobuf::TimestampView, Timestamp};

struct FailingReader;

impl Read for FailingReader {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::NotFound))
    }
}

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::from(io::ErrorKind::PermissionDenied))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct ZeroWriter;

impl Write for ZeroWriter {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Ok(0)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn from_reader_keeps_io_error_kind() {
    let error = buffa_yaml::from_reader::<_, Timestamp>(FailingReader).unwrap_err();
    assert_eq!(error.io_error_kind(), Some(io::ErrorKind::NotFound));
}

#[test]
fn to_writer_keeps_io_error_kind() {
    let error = buffa_yaml::to_writer(FailingWriter, &Timestamp::default()).unwrap_err();
    assert_eq!(error.io_error_kind(), Some(io::ErrorKind::PermissionDenied));
}

#[test]
fn to_writer_keeps_write_zero_error_kind() {
    let error = buffa_yaml::to_writer(ZeroWriter, &Timestamp::default()).unwrap_err();
    assert_eq!(error.io_error_kind(), Some(io::ErrorKind::WriteZero));
}

#[test]
fn to_writer_view_keeps_write_zero_error_kind() {
    use buffa::{Message as _, MessageView as _};

    let bytes = Timestamp::default().encode_to_vec();
    let view = TimestampView::decode_view(&bytes).unwrap();
    let error = buffa_yaml::to_writer_view(ZeroWriter, &view).unwrap_err();
    assert_eq!(error.io_error_kind(), Some(io::ErrorKind::WriteZero));
}

#[test]
fn parse_error_has_no_io_error_kind() {
    let error = buffa_yaml::from_str::<Timestamp>("[").unwrap_err();
    assert_eq!(error.io_error_kind(), None);
    assert!(std::error::Error::source(&error).is_none());
}
