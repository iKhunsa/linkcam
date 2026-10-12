//! Strongly typed hardware units.
//!
//! Pan/tilt use 1/3600-degree UVC units; the firmware rounds every write
//! to a multiple of 3600 (1 degree). Zoom uses the UVC `zoom-abs` scale,
//! where 100 is 1x. Values come from linkcam (Link 2); the Link 2C range
//! is read from the device when available (see `PtzLimits`).

use crate::error::{CameraError, Result};

pub const ANGLE_STEP_UNITS: i32 = 3600;
pub const ZOOM_UNITS_PER_X: f64 = 100.0;

/// Pan or tilt angle in raw 1/3600-degree units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct AngleUnits(i32);

impl AngleUnits {
    pub const ZERO: Self = Self(0);

    /// Builds an angle from raw units, clamped to `range` and snapped to whole degrees.
    pub fn from_raw_clamped(raw: i64, range: AngleRange) -> Self {
        let clamped = raw.clamp(range.min as i64, range.max as i64);
        let steps = (clamped as f64 / ANGLE_STEP_UNITS as f64).round() as i64;
        let snapped = (steps * ANGLE_STEP_UNITS as i64).clamp(range.min as i64, range.max as i64);
        Self(snapped as i32)
    }

    /// Builds an angle without range checks, for values read back from the camera.
    pub const fn from_raw(raw: i32) -> Self {
        Self(raw)
    }

    pub fn from_degrees(degrees: f64, range: AngleRange) -> Self {
        Self::from_raw_clamped((degrees * ANGLE_STEP_UNITS as f64).round() as i64, range)
    }

    pub const fn raw(self) -> i32 {
        self.0
    }

    pub fn degrees(self) -> f64 {
        self.0 as f64 / ANGLE_STEP_UNITS as f64
    }
}

/// Inclusive range of valid raw angle values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AngleRange {
    pub min: i32,
    pub max: i32,
}

impl AngleRange {
    pub const fn degrees(min: i32, max: i32) -> Self {
        Self { min: min * ANGLE_STEP_UNITS, max: max * ANGLE_STEP_UNITS }
    }
}

/// Absolute zoom in UVC units (100 = 1x).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ZoomUnits(u16);

impl ZoomUnits {
    pub const ONE_X: Self = Self(100);

    pub fn from_raw_clamped(raw: i64, range: ZoomRange) -> Self {
        Self(raw.clamp(range.min as i64, range.max as i64) as u16)
    }

    pub fn from_multiplier(multiplier: f64, range: ZoomRange) -> Self {
        Self::from_raw_clamped((multiplier * ZOOM_UNITS_PER_X).round() as i64, range)
    }

    pub const fn from_raw(raw: u16) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u16 {
        self.0
    }

    pub fn multiplier(self) -> f64 {
        self.0 as f64 / ZOOM_UNITS_PER_X
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoomRange {
    pub min: u16,
    pub max: u16,
}

/// Mechanical/digital limits of one camera model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtzLimits {
    pub pan: AngleRange,
    pub tilt: AngleRange,
    pub zoom: ZoomRange,
}

impl PtzLimits {
    /// Measured on the Link 2 gimbal by linkcam. The Link 2C is a fixed camera;
    /// its digital pan/tilt range is replaced by the value reported by the device.
    pub const LINK_2: Self = Self {
        pan: AngleRange::degrees(-145, 145),
        tilt: AngleRange::degrees(-90, 100),
        zoom: ZoomRange { min: 100, max: 400 },
    };
}

/// Parses `30`, `-12.5`, `30deg`, `30d` (degrees) or `108000u` (raw units).
pub fn parse_angle(input: &str, range: AngleRange) -> Result<AngleUnits> {
    let lower = input.trim().to_ascii_lowercase();
    let (number, raw_units) = if let Some(n) = lower.strip_suffix("deg") {
        (n, false)
    } else if let Some(n) = lower.strip_suffix('d') {
        (n, false)
    } else if let Some(n) = lower.strip_suffix('u') {
        (n, true)
    } else {
        (lower.as_str(), false)
    };
    let value = parse_finite(number, input)?;
    let raw = if raw_units { value } else { value * ANGLE_STEP_UNITS as f64 };
    Ok(AngleUnits::from_raw_clamped(raw.round() as i64, range))
}

/// Parses a zoom multiplier (`1.5`, `1.5x`) or raw UVC units (`150`).
///
/// The two ranges do not overlap: values up to 4 are multipliers, values
/// from 100 up are raw units. Anything in between is ambiguous and rejected.
pub fn parse_zoom(input: &str, range: ZoomRange) -> Result<ZoomUnits> {
    let text = input.trim().to_ascii_lowercase();
    let (number, explicit_multiplier) = match text.strip_suffix('x') {
        Some(n) => (n, true),
        None => (text.as_str(), false),
    };
    let value = parse_finite(number, input)?;
    let raw = if explicit_multiplier || value <= 4.0 {
        value * ZOOM_UNITS_PER_X
    } else if value >= range.min as f64 {
        value
    } else {
        return Err(CameraError::InvalidArgument(format!("ambiguous zoom `{input}`")));
    };
    Ok(ZoomUnits::from_raw_clamped(raw.round() as i64, range))
}

fn parse_finite(number: &str, original: &str) -> Result<f64> {
    number
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| CameraError::InvalidArgument(format!("not a number: `{original}`")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: PtzLimits = PtzLimits::LINK_2;

    #[test]
    fn angle_snaps_to_whole_degrees() {
        assert_eq!(AngleUnits::from_raw_clamped(1000, L.pan).raw(), 0);
        assert_eq!(AngleUnits::from_raw_clamped(1900, L.pan).raw(), 3600);
    }

    #[test]
    fn angle_is_clamped_to_range() {
        assert_eq!(AngleUnits::from_degrees(999.0, L.pan).degrees(), 145.0);
        assert_eq!(AngleUnits::from_degrees(-999.0, L.tilt).degrees(), -90.0);
    }

    #[test]
    fn parse_angle_accepts_degrees_and_raw() {
        assert_eq!(parse_angle("30", L.pan).unwrap().raw(), 108_000);
        assert_eq!(parse_angle("-12.5deg", L.pan).unwrap().degrees(), -13.0);
        assert_eq!(parse_angle("108000u", L.pan).unwrap().raw(), 108_000);
        assert!(parse_angle("abc", L.pan).is_err());
    }

    #[test]
    fn parse_zoom_matches_linkcam_rules() {
        assert_eq!(parse_zoom("1.5", L.zoom).unwrap().raw(), 150);
        assert_eq!(parse_zoom("1.5x", L.zoom).unwrap().raw(), 150);
        assert_eq!(parse_zoom("150", L.zoom).unwrap().raw(), 150);
        assert_eq!(parse_zoom("9x", L.zoom).unwrap().raw(), 400);
        assert!(parse_zoom("50", L.zoom).is_err());
        assert!(parse_zoom("nan", L.zoom).is_err());
    }
}
