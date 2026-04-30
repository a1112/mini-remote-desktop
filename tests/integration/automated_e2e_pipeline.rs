mod e2e_support;

use anyhow::Result;
use e2e_support::{run_pipeline_case, E2ePipelineCase};

#[test]
fn synthetic_capture_encode_quic_decode_render_pipeline() -> Result<()> {
    let report = run_pipeline_case(&E2ePipelineCase {
        name: "baseline_640x360_30fps",
        width: 640,
        height: 360,
        fps: 30,
        frame_count: 18,
        bitrate_bps: 4_000_000,
        mtu: 1200,
    })?;

    assert!(report.encoded_access_units > 0);
    assert!(report.transported_access_units > 0);
    assert!(report.decoded_frames > 0);
    assert_eq!(report.decoded_frames, report.rendered_frames);
    assert_eq!(report.width, 640);
    assert_eq!(report.height, 360);

    Ok(())
}
