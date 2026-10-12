//! Rule-based static pose classification from 21 hand landmarks.
//!
//! Rules compare distances to the wrist, so they do not depend on hand
//! orientation or camera distance.

use crate::geometry::{idx, Hand};
use linkcam_core::gesture::Gesture;

/// A finger counts as extended when its tip is clearly farther from the
/// wrist than its middle joint.
const EXTENDED_RATIO: f32 = 1.12;
/// Thumb tip must be this much farther from the index knuckle than the thumb joint.
const THUMB_OUT_RATIO: f32 = 1.25;
/// Thumb and index tips closer than this fraction of the hand size form a pinch.
const PINCH_FRACTION: f32 = 0.30;
/// A thumbs-up must point upwards by at least this fraction of the hand size.
const THUMB_UP_FRACTION: f32 = 0.6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fingers {
    thumb: bool,
    index: bool,
    middle: bool,
    ring: bool,
    pinky: bool,
}

impl Fingers {
    fn of(hand: &Hand) -> Self {
        let wrist = hand.wrist();
        let lm = &hand.landmarks;
        let extended =
            |tip: usize, pip: usize| lm[tip].distance(wrist) > lm[pip].distance(wrist) * EXTENDED_RATIO;
        let thumb = lm[idx::THUMB_TIP].distance(lm[idx::INDEX_MCP])
            > lm[idx::THUMB_IP].distance(lm[idx::INDEX_MCP]) * THUMB_OUT_RATIO;
        Self {
            thumb,
            index: extended(idx::INDEX_TIP, idx::INDEX_PIP),
            middle: extended(idx::MIDDLE_TIP, idx::MIDDLE_PIP),
            ring: extended(idx::RING_TIP, idx::RING_PIP),
            pinky: extended(idx::PINKY_TIP, idx::PINKY_PIP),
        }
    }

    fn four_extended(self) -> bool {
        self.index && self.middle && self.ring && self.pinky
    }

    fn none_of_four_extended(self) -> bool {
        !self.index && !self.middle && !self.ring && !self.pinky
    }
}

/// Classifies the static pose of `hand`; `None` when no known pose matches.
pub fn classify(hand: &Hand) -> Option<Gesture> {
    let fingers = Fingers::of(hand);
    let lm = &hand.landmarks;
    let size = hand.size();

    let pinch = lm[idx::THUMB_TIP].distance(lm[idx::INDEX_TIP]) < size * PINCH_FRACTION;
    if pinch && (fingers.middle || fingers.ring || fingers.pinky) {
        return Some(Gesture::Pinch);
    }
    if fingers.four_extended() {
        return Some(Gesture::OpenPalm);
    }
    if fingers.index && fingers.middle && !fingers.ring && !fingers.pinky {
        return Some(Gesture::Peace);
    }
    if fingers.index && !fingers.middle && !fingers.ring && !fingers.pinky {
        return Some(Gesture::Point);
    }
    if fingers.none_of_four_extended() {
        let thumb_points_up = hand.wrist().y - lm[idx::THUMB_TIP].y > size * THUMB_UP_FRACTION;
        return if fingers.thumb && thumb_points_up { Some(Gesture::ThumbsUp) } else { Some(Gesture::Fist) };
    }
    None
}

#[cfg(test)]
pub(crate) mod fixtures {
    use crate::geometry::{Hand, Landmark, HAND_LANDMARKS};

    /// Builds an upright right hand (wrist at the bottom) from per-finger extension flags.
    /// Each finger is a straight line of 4 joints; folded fingers curl back toward the palm.
    pub fn hand(thumb: bool, index: bool, middle: bool, ring: bool, pinky: bool) -> Hand {
        let mut lm = [Landmark::default(); HAND_LANDMARKS];
        lm[0] = Landmark { x: 0.5, y: 0.9 };
        // (base x, knuckle y) of the four long fingers; joints 0..3 go from MCP to TIP.
        let fingers = [(0.44, index, 5), (0.5, middle, 9), (0.56, ring, 13), (0.62, pinky, 17)];
        for (x, extended, first) in fingers {
            for joint in 0..4 {
                let y = if extended {
                    0.7 - 0.1 * joint as f32
                } else {
                    // folded: tip comes back down near the knuckle
                    [0.7, 0.62, 0.66, 0.74][joint]
                };
                lm[first + joint] = Landmark { x, y };
            }
        }
        // Thumb chain 1..=4 sits to the left of the palm.
        for joint in 0..4 {
            let (x, y) = if thumb {
                (0.40 - 0.05 * joint as f32, 0.78 - 0.08 * joint as f32)
            } else {
                (0.42 - 0.005 * joint as f32, 0.78 - 0.01 * joint as f32)
            };
            lm[1 + joint] = Landmark { x, y };
        }
        Hand { landmarks: lm, confidence: 1.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::hand;
    use super::*;

    #[test]
    fn open_palm() {
        assert_eq!(classify(&hand(true, true, true, true, true)), Some(Gesture::OpenPalm));
    }

    #[test]
    fn peace_sign() {
        assert_eq!(classify(&hand(false, true, true, false, false)), Some(Gesture::Peace));
    }

    #[test]
    fn pointing() {
        assert_eq!(classify(&hand(false, true, false, false, false)), Some(Gesture::Point));
    }

    #[test]
    fn fist_without_thumb_out() {
        assert_eq!(classify(&hand(false, false, false, false, false)), Some(Gesture::Fist));
    }

    #[test]
    fn thumbs_up_needs_thumb_out_and_up() {
        assert_eq!(classify(&hand(true, false, false, false, false)), Some(Gesture::ThumbsUp));
    }

    #[test]
    fn ambiguous_pose_is_unknown() {
        assert_eq!(classify(&hand(true, false, true, true, false)), None);
    }
}
