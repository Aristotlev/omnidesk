//! Captures screen 0 once and writes it to the given .jpg file. Handy for checking capture on a new machine.
use omnidesk_platform::native;
use omnidesk_proto::StreamOptions;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "screen.jpg".into());
    let platform = native();
    println!(
        "{} ({:?}), screens: {:#?}",
        platform.device_name(),
        platform.kind(),
        platform.screens()
    );
    match platform.capture(0, StreamOptions::FOCUS) {
        Ok(frame) => {
            std::fs::write(&path, &frame.data).expect("write file");
            println!("wrote {}x{} to {path}", frame.width, frame.height);
        }
        Err(e) => println!("capture failed: {e}"),
    }
}
