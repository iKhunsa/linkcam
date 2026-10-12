//! Video stream formats (resolution + frame rate) the camera can output.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct VideoFormat {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl VideoFormat {
    pub const fn new(width: u32, height: u32, fps: u32) -> Self {
        Self { width, height, fps }
    }

    /// Short name as shown by the official controller: `4K30`, `1080p60`, `720p30`.
    pub fn label(self) -> String {
        let name = match self.height {
            2160 => "4K".to_string(),
            h => format!("{h}p"),
        };
        format!("{name}{}", self.fps)
    }

    /// Orders formats from largest to smallest, then by frame rate descending.
    pub fn sorted_descending(mut formats: Vec<Self>) -> Vec<Self> {
        formats.sort_by_key(|f| std::cmp::Reverse((f.width, f.height, f.fps)));
        formats.dedup();
        formats
    }

    /// Picks the entry of `available` closest to `wanted`: same size first,
    /// then the highest frame rate not above the wanted one.
    pub fn closest(available: &[Self], wanted: Self) -> Option<Self> {
        let same_size: Vec<Self> = available
            .iter()
            .copied()
            .filter(|f| f.width == wanted.width && f.height == wanted.height)
            .collect();
        let pool = if same_size.is_empty() { available } else { &same_size };
        pool.iter().copied().min_by_key(|f| {
            let size_gap = f.width.abs_diff(wanted.width) + f.height.abs_diff(wanted.height);
            let over = f.fps > wanted.fps;
            (size_gap, over, f.fps.abs_diff(wanted.fps))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_match_the_official_style() {
        assert_eq!(VideoFormat::new(3840, 2160, 30).label(), "4K30");
        assert_eq!(VideoFormat::new(1920, 1080, 60).label(), "1080p60");
        assert_eq!(VideoFormat::new(1280, 720, 30).label(), "720p30");
    }

    #[test]
    fn sorted_largest_first_without_duplicates() {
        let sorted = VideoFormat::sorted_descending(vec![
            VideoFormat::new(1280, 720, 30),
            VideoFormat::new(3840, 2160, 30),
            VideoFormat::new(1280, 720, 30),
            VideoFormat::new(1920, 1080, 60),
            VideoFormat::new(1920, 1080, 30),
        ]);
        assert_eq!(sorted[0].label(), "4K30");
        assert_eq!(sorted.len(), 4);
        assert_eq!(sorted[1].label(), "1080p60");
    }

    #[test]
    fn closest_prefers_same_size_and_not_exceeding_fps() {
        let available = [
            VideoFormat::new(1920, 1080, 60),
            VideoFormat::new(1920, 1080, 30),
            VideoFormat::new(1280, 720, 30),
        ];
        let got = VideoFormat::closest(&available, VideoFormat::new(1920, 1080, 30)).unwrap();
        assert_eq!(got.fps, 30);
        let fallback = VideoFormat::closest(&available, VideoFormat::new(3840, 2160, 30)).unwrap();
        assert_eq!((fallback.width, fallback.height), (1920, 1080));
    }
}
