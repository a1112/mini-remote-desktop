use mrd_pipeline_core::VideoEncoder;

use super::media_access_unit::LanAccessUnitCodec;

pub(super) struct LanSenderEncoder {
    pub(super) codec: LanAccessUnitCodec,
    pub(super) backend: &'static str,
    pub(super) encoder: Box<dyn VideoEncoder + Send>,
}
