/// Camera video modes selectable through the vendor extension unit
/// (node 1, selector 2). Ids and flags were measured on a Link 2C; see
/// `docs/xu-map-link2c.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoMode {
    Normal,
    Whiteboard,
    Overhead,
    DeskView,
    /// Id and flag confirmed by driving the official controller (see the XU map).
    AutoFraming,
}

impl VideoMode {
    pub const ALL: [Self; 5] =
        [Self::Normal, Self::Whiteboard, Self::Overhead, Self::DeskView, Self::AutoFraming];

    /// Value of payload byte 0.
    pub const fn id(self) -> u8 {
        match self {
            Self::Normal => 0,
            Self::Whiteboard => 4,
            Self::Overhead => 5,
            Self::DeskView => 6,
            Self::AutoFraming => 7,
        }
    }

    /// Value of payload byte 1 required for the mode to be accepted.
    pub const fn flag(self) -> u8 {
        match self {
            Self::DeskView | Self::AutoFraming => 0x10,
            Self::Whiteboard => 0x01,
            Self::Normal | Self::Overhead => 0x00,
        }
    }

    pub const fn from_id(id: u8) -> Option<Self> {
        match id {
            0 => Some(Self::Normal),
            4 => Some(Self::Whiteboard),
            5 => Some(Self::Overhead),
            6 => Some(Self::DeskView),
            7 => Some(Self::AutoFraming),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Whiteboard => "whiteboard",
            Self::Overhead => "overhead",
            Self::DeskView => "deskview",
            Self::AutoFraming => "auto-framing",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.label().eq_ignore_ascii_case(label))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_and_label_roundtrip() {
        for mode in VideoMode::ALL {
            assert_eq!(VideoMode::from_id(mode.id()), Some(mode));
            assert_eq!(VideoMode::from_label(mode.label()), Some(mode));
        }
        assert_eq!(VideoMode::from_id(0xff), None);
    }

    #[test]
    fn deskview_requires_its_flag() {
        assert_eq!(VideoMode::DeskView.flag(), 0x10);
        assert_eq!(VideoMode::Normal.flag(), 0);
        assert_eq!(VideoMode::Whiteboard.flag(), 0x01);
    }
}
