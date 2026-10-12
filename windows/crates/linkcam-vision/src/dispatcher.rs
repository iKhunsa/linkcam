//! Turns a stream of per-frame observations into fired bindings.
//!
//! * Static poses fire once after being held for the binding's `hold_ms`,
//!   then must be released (or changed) before they can fire again.
//! * Swipes are detected from the wrist trajectory over a short window.
//! * Every binding has a cooldown.
//!
//! Directions are from the *user's* point of view: the camera faces the user,
//! so a hand moving to the user's right moves to the left of the raw image.

use crate::geometry::Landmark;
use linkcam_core::gesture::{Action, Binding, Gesture};
use std::collections::VecDeque;

/// Trajectory window used to recognise a swipe.
const SWIPE_WINDOW_MS: u64 = 500;
/// Minimum travel (fraction of the frame) for a swipe.
const SWIPE_MIN_TRAVEL: f32 = 0.28;
/// The main axis must dominate the other by this factor.
const SWIPE_AXIS_DOMINANCE: f32 = 2.0;

/// A binding that fired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fired {
    pub gesture: Gesture,
    pub action: Action,
}

struct Slot {
    binding: Binding,
    last_fired_ms: Option<u64>,
    fired_in_current_hold: bool,
}

pub struct Dispatcher {
    slots: Vec<Slot>,
    pose: Option<Gesture>,
    pose_since_ms: u64,
    trail: VecDeque<(u64, Landmark)>,
}

impl Dispatcher {
    pub fn new(bindings: &[Binding]) -> Self {
        let slots = bindings
            .iter()
            .filter(|b| b.enabled)
            .map(|b| Slot { binding: b.clone(), last_fired_ms: None, fired_in_current_hold: false })
            .collect();
        Self { slots, pose: None, pose_since_ms: 0, trail: VecDeque::new() }
    }

    /// Feeds one observation. `pose`/`wrist` are `None` when no hand is visible.
    pub fn update(&mut self, now_ms: u64, pose: Option<Gesture>, wrist: Option<Landmark>) -> Vec<Fired> {
        self.track_pose(now_ms, pose);
        self.track_trail(now_ms, wrist);

        let mut fired = Vec::new();
        if let Some(swipe) = self.detect_swipe() {
            self.trail.clear();
            self.fire_matching(now_ms, swipe, |_| true, &mut fired);
        }
        if let Some(pose) = self.pose.filter(|p| !p.is_motion()) {
            let held_ms = now_ms.saturating_sub(self.pose_since_ms);
            self.fire_matching(now_ms, pose, |slot| held_ms >= slot.binding.hold_ms as u64, &mut fired);
        }
        fired
    }

    fn track_pose(&mut self, now_ms: u64, pose: Option<Gesture>) {
        if pose != self.pose {
            self.pose = pose;
            self.pose_since_ms = now_ms;
            for slot in &mut self.slots {
                slot.fired_in_current_hold = false;
            }
        }
    }

    fn track_trail(&mut self, now_ms: u64, wrist: Option<Landmark>) {
        match wrist {
            Some(point) => {
                self.trail.push_back((now_ms, point));
                while self.trail.front().is_some_and(|(t, _)| now_ms.saturating_sub(*t) > SWIPE_WINDOW_MS) {
                    self.trail.pop_front();
                }
            }
            None => self.trail.clear(),
        }
    }

    fn detect_swipe(&self) -> Option<Gesture> {
        let (_, first) = self.trail.front()?;
        let (_, last) = self.trail.back()?;
        let (dx, dy) = (last.x - first.x, last.y - first.y);
        if dx.abs() >= SWIPE_MIN_TRAVEL && dx.abs() > dy.abs() * SWIPE_AXIS_DOMINANCE {
            // Raw image x decreasing == hand moving to the user's right.
            return Some(if dx < 0.0 { Gesture::SwipeRight } else { Gesture::SwipeLeft });
        }
        if dy.abs() >= SWIPE_MIN_TRAVEL && dy.abs() > dx.abs() * SWIPE_AXIS_DOMINANCE {
            return Some(if dy < 0.0 { Gesture::SwipeUp } else { Gesture::SwipeDown });
        }
        None
    }

    fn fire_matching(
        &mut self,
        now_ms: u64,
        gesture: Gesture,
        ready: impl Fn(&Slot) -> bool,
        out: &mut Vec<Fired>,
    ) {
        for slot in self.slots.iter_mut().filter(|s| s.binding.gesture == gesture) {
            let cooled_down = slot
                .last_fired_ms
                .is_none_or(|t| now_ms.saturating_sub(t) >= slot.binding.cooldown_ms as u64);
            let first_time_this_hold = gesture.is_motion() || !slot.fired_in_current_hold;
            if cooled_down && first_time_this_hold && ready(slot) {
                slot.last_fired_ms = Some(now_ms);
                slot.fired_in_current_hold = true;
                out.push(Fired { gesture, action: slot.binding.action.clone() });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use linkcam_core::gesture::CameraCommand;

    fn keys(chord: &[&str]) -> Action {
        Action::Keys { keys: chord.iter().map(|k| k.to_string()).collect() }
    }

    fn binding(gesture: Gesture, hold_ms: u32, cooldown_ms: u32) -> Binding {
        Binding { gesture, action: keys(&["x"]), hold_ms, cooldown_ms, enabled: true }
    }

    #[test]
    fn pose_fires_once_after_hold_time() {
        let mut d = Dispatcher::new(&[binding(Gesture::Peace, 500, 1000)]);
        let mut total = 0;
        for t in (0..=2000).step_by(100) {
            total += d.update(t, Some(Gesture::Peace), None).len();
        }
        assert_eq!(total, 1, "must fire exactly once while the pose is held");
    }

    #[test]
    fn releasing_and_remaking_the_pose_fires_again_after_cooldown() {
        let mut d = Dispatcher::new(&[binding(Gesture::Fist, 200, 500)]);
        let mut total = 0;
        for t in (0..=400).step_by(100) {
            total += d.update(t, Some(Gesture::Fist), None).len();
        }
        d.update(500, None, None);
        for t in (1000..=1400).step_by(100) {
            total += d.update(t, Some(Gesture::Fist), None).len();
        }
        assert_eq!(total, 2);
    }

    #[test]
    fn short_pose_does_not_fire() {
        let mut d = Dispatcher::new(&[binding(Gesture::ThumbsUp, 600, 1000)]);
        let fired: usize =
            (0..=400).step_by(100).map(|t| d.update(t, Some(Gesture::ThumbsUp), None).len()).sum();
        assert_eq!(fired, 0);
    }

    #[test]
    fn hand_moving_left_in_the_image_is_a_right_swipe_for_the_user() {
        let mut d = Dispatcher::new(&[binding(Gesture::SwipeRight, 0, 800)]);
        let mut fired = Vec::new();
        for (i, x) in [0.8, 0.7, 0.6, 0.5, 0.4].into_iter().enumerate() {
            fired.extend(d.update(i as u64 * 100, None, Some(Landmark { x, y: 0.5 })));
        }
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].gesture, Gesture::SwipeRight);
    }

    #[test]
    fn diagonal_motion_is_not_a_swipe() {
        let mut d = Dispatcher::new(&[binding(Gesture::SwipeRight, 0, 0), binding(Gesture::SwipeDown, 0, 0)]);
        let mut count = 0;
        for i in 0..5u64 {
            let p = Landmark { x: 0.8 - 0.08 * i as f32, y: 0.3 + 0.08 * i as f32 };
            count += d.update(i * 100, None, Some(p)).len();
        }
        assert_eq!(count, 0);
    }

    #[test]
    fn disabled_bindings_never_fire() {
        let mut off = binding(Gesture::Fist, 0, 0);
        off.enabled = false;
        off.action = Action::Camera(CameraCommand::Recenter);
        let mut d = Dispatcher::new(&[off]);
        assert!(d.update(0, Some(Gesture::Fist), None).is_empty());
        assert!(d.update(1000, Some(Gesture::Fist), None).is_empty());
    }
}
