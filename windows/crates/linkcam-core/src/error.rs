use std::fmt;

/// Everything that can go wrong while talking to the camera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CameraError {
    DeviceNotFound,
    ControlUnavailable(&'static str),
    ReadFailed(String),
    WriteFailed(String),
    InvalidArgument(String),
}

impl fmt::Display for CameraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceNotFound => {
                write!(f, "Insta360 Link 2C not found; check that it is plugged in")
            }
            Self::ControlUnavailable(name) => {
                write!(f, "the camera does not expose the control `{name}`")
            }
            Self::ReadFailed(why) => write!(f, "could not read from the camera: {why}"),
            Self::WriteFailed(why) => write!(f, "the camera rejected the update: {why}"),
            Self::InvalidArgument(why) => write!(f, "invalid argument: {why}"),
        }
    }
}

impl std::error::Error for CameraError {}

pub type Result<T> = std::result::Result<T, CameraError>;
