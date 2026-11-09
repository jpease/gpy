use super::{ConnectErrorClass, connect_error_class};
use std::io::{Error as IoError, ErrorKind};

#[test]
fn refused_and_missing_mean_not_listening() {
    for kind in [ErrorKind::ConnectionRefused, ErrorKind::NotFound] {
        assert_eq!(
            connect_error_class(&IoError::from(kind)),
            ConnectErrorClass::NotListening,
            "{kind:?}"
        );
    }
}

#[test]
fn would_block_means_the_connect_queued() {
    assert_eq!(
        connect_error_class(&IoError::from(ErrorKind::WouldBlock)),
        ConnectErrorClass::Queued
    );
}

#[test]
fn a_path_that_is_no_socket_means_not_listening() {
    assert_eq!(
        connect_error_class(&IoError::from_raw_os_error(libc::ENOTSOCK)),
        ConnectErrorClass::NotListening
    );
}

#[test]
fn permission_errors_are_local_failures() {
    for kind in [ErrorKind::PermissionDenied, ErrorKind::Other] {
        assert_eq!(
            connect_error_class(&IoError::from(kind)),
            ConnectErrorClass::Local,
            "{kind:?}"
        );
    }
}
