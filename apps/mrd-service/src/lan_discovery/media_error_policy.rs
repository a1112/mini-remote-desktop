pub(super) const LAN_MEDIA_SENDER_MAX_CONSECUTIVE_FRAME_ERRORS: u32 = 8;
pub(super) const LAN_MEDIA_RECEIVER_MAX_CONSECUTIVE_DECODE_ERRORS: u32 = 8;

const LAN_MEDIA_SENDER_ERROR_LOG_INTERVAL: u32 = 3;
const LAN_MEDIA_RECEIVER_DECODE_ERROR_LOG_INTERVAL: u32 = 3;

pub(super) fn should_log_media_sender_frame_error(consecutive_frame_errors: u32) -> bool {
    consecutive_frame_errors == 1
        || consecutive_frame_errors >= LAN_MEDIA_SENDER_MAX_CONSECUTIVE_FRAME_ERRORS
        || consecutive_frame_errors.is_multiple_of(LAN_MEDIA_SENDER_ERROR_LOG_INTERVAL)
}

pub(super) fn should_log_media_receiver_decode_error(consecutive_decode_errors: u32) -> bool {
    consecutive_decode_errors == 1
        || consecutive_decode_errors == LAN_MEDIA_RECEIVER_MAX_CONSECUTIVE_DECODE_ERRORS
        || consecutive_decode_errors.is_multiple_of(LAN_MEDIA_RECEIVER_DECODE_ERROR_LOG_INTERVAL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sender_frame_errors_log_first_interval_and_terminal_counts() {
        assert!(should_log_media_sender_frame_error(1));
        assert!(!should_log_media_sender_frame_error(2));
        assert!(should_log_media_sender_frame_error(3));
        assert!(!should_log_media_sender_frame_error(4));
        assert!(should_log_media_sender_frame_error(
            LAN_MEDIA_SENDER_MAX_CONSECUTIVE_FRAME_ERRORS
        ));
        assert!(should_log_media_sender_frame_error(
            LAN_MEDIA_SENDER_MAX_CONSECUTIVE_FRAME_ERRORS + 1
        ));
    }

    #[test]
    fn receiver_decode_errors_log_first_interval_and_threshold_count() {
        assert!(should_log_media_receiver_decode_error(1));
        assert!(!should_log_media_receiver_decode_error(2));
        assert!(should_log_media_receiver_decode_error(3));
        assert!(!should_log_media_receiver_decode_error(4));
        assert!(should_log_media_receiver_decode_error(
            LAN_MEDIA_RECEIVER_MAX_CONSECUTIVE_DECODE_ERRORS
        ));
        assert!(should_log_media_receiver_decode_error(9));
    }
}
