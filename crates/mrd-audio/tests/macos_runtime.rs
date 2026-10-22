#![cfg(target_os = "macos")]
use mrd_audio::{
    macos::{AudioPlayback, SystemAudioCapture},
    AudioPacket,
};
use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// Explicit native acceptance probe: plays a macOS system sound, captures it
/// through ScreenCaptureKit, validates wire frames and submits them to CoreAudio.
#[test]
#[ignore = "plays a test sound; requires a logged-in macOS desktop and screen recording permission"]
fn system_sound_capture_wire_roundtrip_and_native_playback() {
    let capture = SystemAudioCapture::start([11; 16]).expect("ScreenCaptureKit audio capture");
    let playback =
        AudioPlayback::start(Instant::now() + Duration::from_secs(8)).expect("CoreAudio playback");
    let mut sound = Command::new("afplay")
        .arg("/System/Library/Sounds/Glass.aiff")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("test sound");
    let deadline = Instant::now() + Duration::from_secs(4);
    let mut packets = 0_u64;
    let mut peak = 0_i32;
    while Instant::now() < deadline {
        if let Ok(captured) = capture.packets.recv_timeout(Duration::from_millis(50)) {
            let bytes = captured.packet.encode().unwrap();
            let packet = AudioPacket::decode(&bytes, [11; 16]).unwrap();
            peak = peak.max(
                packet
                    .samples
                    .iter()
                    .map(|sample| i32::from(*sample).abs())
                    .max()
                    .unwrap_or(0),
            );
            playback.buffer.lock().unwrap().push(packet);
            packets += 1;
        }
    }
    let _ = sound.kill();
    let _ = sound.wait();
    let buffer = playback.buffer.lock().unwrap();
    println!("system_audio packets={packets} peak_pcm16={peak} rendered_samples={} dropped_packets={} underruns={}", buffer.rendered_samples, buffer.dropped_packets, buffer.underrun_packets);
    assert!(packets > 20, "audio capture produced too few packets");
    assert!(peak > 100, "captured audio was silent");
    assert!(
        buffer.rendered_samples > 1920,
        "native output callback did not consume PCM"
    );
    assert!(
        playback.error.lock().unwrap().is_none(),
        "CoreAudio reported an output failure"
    );
}
