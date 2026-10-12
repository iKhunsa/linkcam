//! Runs the hand pipeline on PNG files: `cargo run -p linkcam-vision --example detect -- <models-dir> <a.png> [b.png...]`

use linkcam_vision::{classify, HandModels, ImageView};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn load_rgba(path: &Path) -> (usize, usize, Vec<u8>) {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg")) {
        let mut decoder =
            jpeg_decoder::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).expect("open jpg")));
        let pixels = decoder.decode().expect("decode jpg");
        let info = decoder.info().expect("jpg info");
        let rgba = pixels.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect();
        return (info.width as usize, info.height as usize, rgba);
    }
    let decoder = png::Decoder::new(std::fs::File::open(path).expect("open png"));
    let mut reader = decoder.read_info().expect("png header");
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).expect("png frame");
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buffer[..w * h * 4].to_vec(),
        png::ColorType::Rgb => {
            buffer[..w * h * 3].as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect()
        }
        other => panic!("unsupported png colour type {other:?}"),
    };
    (w, h, rgba)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dir, images) = args.split_first().expect("usage: detect <models-dir> <png>...");
    let started = Instant::now();
    let models = HandModels::load(&PathBuf::from(dir)).expect("load models");
    println!("models loaded in {:?}", started.elapsed());
    for path in images {
        let (width, height, rgba) = load_rgba(Path::new(path));
        let view = ImageView { width, height, rgba: &rgba };
        let started = Instant::now();
        let hand = match std::env::var("LINKCAM_ROI") {
            Ok(roi) => {
                let n: Vec<f32> = roi.split(',').map(|v| v.parse().unwrap()).collect();
                models
                    .landmarks_in_roi(
                        &view,
                        &linkcam_vision::Roi { center_x: n[0], center_y: n[1], side: n[2], rotation: n[3] },
                    )
                    .expect("inference")
            }
            Err(_) => models.detect(&view).expect("inference"),
        };
        let elapsed = started.elapsed();
        match hand {
            Some(hand) => println!(
                "{path}: hand conf={:.2} gesture={:?} wrist=({:.2},{:.2}) [{elapsed:?}]",
                hand.confidence,
                classify(&hand),
                hand.wrist().x,
                hand.wrist().y
            ),
            None => println!("{path}: no hand [{elapsed:?}]"),
        }
    }
}
