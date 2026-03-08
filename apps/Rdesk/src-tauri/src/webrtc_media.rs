#[derive(Debug, Default, Clone)]
pub struct H264AccessUnitAssembler {
    annex_b_buffer: Vec<u8>,
    fua_active: bool,
}

impl H264AccessUnitAssembler {
    pub fn push_rtp_payload(&mut self, payload: &[u8], marker: bool) -> Option<Vec<u8>> {
        if payload.is_empty() {
            return None;
        }

        let nal_type = payload[0] & 0x1f;
        match nal_type {
            1..=23 => {
                self.append_nal(payload);
                if marker {
                    self.take_access_unit()
                } else {
                    None
                }
            }
            24 => self.push_stap_a(payload, marker),
            28 => self.push_fua(payload, marker),
            _ => {
                if marker {
                    self.reset();
                }
                None
            }
        }
    }

    fn push_fua(&mut self, payload: &[u8], marker: bool) -> Option<Vec<u8>> {
        if payload.len() < 2 {
            self.reset();
            return None;
        }

        let fu_indicator = payload[0];
        let fu_header = payload[1];
        let start = fu_header & 0x80 != 0;
        let end = fu_header & 0x40 != 0;
        let reconstructed_nal = (fu_indicator & 0xe0) | (fu_header & 0x1f);

        if start {
            if self.fua_active {
                self.reset();
            }
            self.annex_b_buffer.extend_from_slice(&[0, 0, 0, 1, reconstructed_nal]);
            self.annex_b_buffer.extend_from_slice(&payload[2..]);
            self.fua_active = true;
        } else if self.fua_active {
            self.annex_b_buffer.extend_from_slice(&payload[2..]);
        } else {
            self.reset();
            return None;
        }

        if end || marker {
            self.fua_active = false;
            return self.take_access_unit();
        }

        None
    }

    fn push_stap_a(&mut self, payload: &[u8], marker: bool) -> Option<Vec<u8>> {
        if payload.len() < 3 {
            self.reset();
            return None;
        }

        let mut offset = 1usize;
        while offset + 2 <= payload.len() {
            let nal_len = u16::from_be_bytes([payload[offset], payload[offset + 1]]) as usize;
            offset += 2;
            if offset + nal_len > payload.len() {
                self.reset();
                return None;
            }
            self.append_nal(&payload[offset..offset + nal_len]);
            offset += nal_len;
        }

        if marker {
            return self.take_access_unit();
        }

        None
    }

    fn append_nal(&mut self, nal: &[u8]) {
        self.annex_b_buffer.extend_from_slice(&[0, 0, 0, 1]);
        self.annex_b_buffer.extend_from_slice(nal);
    }

    fn take_access_unit(&mut self) -> Option<Vec<u8>> {
        if self.annex_b_buffer.is_empty() {
            return None;
        }
        let mut complete = Vec::new();
        std::mem::swap(&mut complete, &mut self.annex_b_buffer);
        Some(complete)
    }

    fn reset(&mut self) {
        self.annex_b_buffer.clear();
        self.fua_active = false;
    }
}

#[cfg(test)]
mod tests {
    use super::H264AccessUnitAssembler;

    #[test]
    fn single_nal_emits_annex_b_access_unit_on_marker() {
        let mut assembler = H264AccessUnitAssembler::default();

        let access_unit = assembler
            .push_rtp_payload(&[0x65, 0x88, 0x99], true)
            .expect("single nal access unit");

        assert_eq!(access_unit, vec![0, 0, 0, 1, 0x65, 0x88, 0x99]);
    }

    #[test]
    fn fua_fragments_emit_single_access_unit() {
        let mut assembler = H264AccessUnitAssembler::default();

        assert_eq!(
            assembler.push_rtp_payload(&[0x7c, 0x85, 0xaa, 0xbb], false),
            None
        );
        assert_eq!(
            assembler.push_rtp_payload(&[0x7c, 0x45, 0xcc, 0xdd], true),
            Some(vec![0, 0, 0, 1, 0x65, 0xaa, 0xbb, 0xcc, 0xdd])
        );
    }

    #[test]
    fn stap_a_then_fua_preserves_full_access_unit_until_marker() {
        let mut assembler = H264AccessUnitAssembler::default();

        assert_eq!(
            assembler.push_rtp_payload(
                &[
                    24,
                    0, 2, 0x67, 0x42,
                    0, 2, 0x68, 0xce,
                ],
                false
            ),
            None
        );
        assert_eq!(
            assembler.push_rtp_payload(&[0x7c, 0x85, 0xaa, 0xbb], false),
            None
        );
        assert_eq!(
            assembler.push_rtp_payload(&[0x7c, 0x45, 0xcc, 0xdd], true),
            Some(vec![
                0, 0, 0, 1, 0x67, 0x42,
                0, 0, 0, 1, 0x68, 0xce,
                0, 0, 0, 1, 0x65, 0xaa, 0xbb, 0xcc, 0xdd,
            ])
        );
    }
}
