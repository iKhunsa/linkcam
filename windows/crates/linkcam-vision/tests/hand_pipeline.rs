//! End to end on real photos: image -> models -> pose -> dispatcher -> fired binding.
//! Skipped (not failed) when the model files are not present.

use linkcam_core::gesture::{Action, Binding, CameraCommand, Gesture};
use linkcam_vision::models::PALM_MODEL_FILE;
use linkcam_vision::{classify, Dispatcher, HandModels, ImageView};
use std::path::{Path, PathBuf};

fn models_dir() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models");
    dir.join(PALM_MODEL_FILE).is_file().then_some(dir)
}

fn load_photo(name: &str) -> (usize, usize, Vec<u8>) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name);
    let file = std::io::BufReader::new(std::fs::File::open(path).expect("photo"));
    let mut decoder = jpeg_decoder::Decoder::new(file);
    let pixels = decoder.decode().expect("decode jpeg");
    let info = decoder.info().expect("jpeg info");
    let rgba = pixels.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect();
    (info.width as usize, info.height as usize, rgba)
}

fn pose_of(models: &HandModels, photo: &str) -> Option<Gesture> {
    let (width, height, rgba) = load_photo(photo);
    let hand = models.detect(&ImageView { width, height, rgba: &rgba }).expect("inference")?;
    classify(&hand)
}

#[test]
fn real_photos_are_classified() {
    let Some(dir) = models_dir() else { return };
    let models = HandModels::load(&dir).expect("models load");
    assert_eq!(pose_of(&models, "open_palm.jpg"), Some(Gesture::OpenPalm));
    assert_eq!(pose_of(&models, "fist.jpg"), Some(Gesture::Fist));
    assert_eq!(pose_of(&models, "point.jpg"), Some(Gesture::Point));
}

#[test]
fn held_open_palm_fires_its_binding_once() {
    let Some(dir) = models_dir() else { return };
    let models = HandModels::load(&dir).expect("models load");
    let (width, height, rgba) = load_photo("open_palm.jpg");
    let view = ImageView { width, height, rgba: &rgba };

    let binding = Binding {
        gesture: Gesture::OpenPalm,
        action: Action::Camera(CameraCommand::Recenter),
        hold_ms: 500,
        cooldown_ms: 2000,
        enabled: true,
    };
    let mut dispatcher = Dispatcher::new(&[binding]);
    let mut fired = 0;
    // Same frame at 10 fps for 2 seconds: a steady open palm.
    for step in 0..20u64 {
        let hand = models.detect(&view).expect("inference");
        let pose = hand.as_ref().and_then(classify);
        fired += dispatcher.update(step * 100, pose, hand.as_ref().map(|h| h.wrist())).len();
    }
    assert_eq!(fired, 1, "a held pose fires exactly once");
}
