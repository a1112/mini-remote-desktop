use mrd_proto::{DeviceId, SessionId};
use mrd_signal_proto::{
    webrtc_candidate_fingerprint_v3, SessionGrantV3, SessionIntentV3, WanSessionRequestV3,
    WebRtcCandidateV3, WebRtcDescriptionRoleV3, WebRtcOfferV3,
};
#[path = "support/browser_fixture.rs"]
mod browser_fixture;
use browser_fixture::{fixture, identity};

#[test]
fn independently_generated_browser_vectors_match_rust_v3_signatures_and_commitments() {
    let fixture = fixture();
    let browser = identity(fixture["browser"]["seed_hex"].as_str().unwrap());
    let target = identity(fixture["target"]["seed_hex"].as_str().unwrap());
    assert_eq!(
        browser.key_id(),
        fixture["browser"]["key_id"].as_str().unwrap()
    );
    assert_eq!(
        target.key_id(),
        fixture["target"]["key_id"].as_str().unwrap()
    );
    let browser_id = DeviceId(fixture["browser"]["device_id"].as_str().unwrap().into());
    let target_id = DeviceId(fixture["target"]["device_id"].as_str().unwrap().into());
    let now = fixture["now_ms"].as_u64().unwrap();

    let request: WanSessionRequestV3 = serde_json::from_value(fixture["request"].clone()).unwrap();
    assert_eq!(
        serde_json::to_string(&request).unwrap(),
        fixture["request_compact"]
    );
    assert_eq!(request.commitment().unwrap(), fixture["request_commitment"]);

    let intent: SessionIntentV3 = serde_json::from_value(fixture["intent"].clone()).unwrap();
    intent.verify_for_without_replay(&target_id, now).unwrap();
    assert_eq!(
        serde_json::to_string(&intent).unwrap(),
        fixture["intent_compact"]
    );
    assert_eq!(intent.commitment().unwrap(), fixture["intent_commitment"]);
    assert_eq!(
        SessionIntentV3::sign(&browser, intent.payload.clone()).unwrap(),
        intent
    );

    let grant: SessionGrantV3 = serde_json::from_value(fixture["grant"].clone()).unwrap();
    grant.verify_for_without_replay(&browser_id, now).unwrap();
    grant.verify_intent(&intent).unwrap();
    assert_eq!(
        serde_json::to_string(&grant).unwrap(),
        fixture["grant_compact"]
    );
    assert_eq!(grant.commitment().unwrap(), fixture["grant_commitment"]);
    assert_eq!(
        SessionGrantV3::sign(&target, grant.payload.clone()).unwrap(),
        grant
    );

    let offer: WebRtcOfferV3 = serde_json::from_value(fixture["offer"].clone()).unwrap();
    offer.verify_for_without_replay(&target_id, now).unwrap();
    offer.verify_grant(&grant).unwrap();
    assert_eq!(
        serde_json::to_string(&offer).unwrap(),
        fixture["offer_compact"]
    );
    assert_eq!(
        WebRtcOfferV3::sign(&browser, offer.payload.clone()).unwrap(),
        offer
    );

    let candidate: WebRtcCandidateV3 =
        serde_json::from_value(fixture["candidate"].clone()).unwrap();
    candidate
        .verify_for_without_replay(&target_id, now)
        .unwrap();
    candidate.verify_grant(&grant).unwrap();
    offer
        .verify_candidate_manifest(&[candidate.payload.clone()])
        .unwrap();
    assert_eq!(
        serde_json::to_string(&candidate).unwrap(),
        fixture["candidate_compact"]
    );
    assert_eq!(
        WebRtcCandidateV3::sign(&browser, candidate.payload.clone()).unwrap(),
        candidate
    );
    assert_eq!(
        webrtc_candidate_fingerprint_v3(
            &SessionId(request.session_id.0),
            fixture["grant_commitment"].as_str().unwrap(),
            WebRtcDescriptionRoleV3::Offer,
            &candidate.payload.candidate,
            candidate.payload.sdp_mid.as_deref(),
            candidate.payload.sdp_mline_index,
            candidate.payload.username_fragment.as_deref(),
        ),
        fixture["candidate_fingerprint"]
    );
}

#[test]
fn browser_signed_v3_wire_rejects_duplicate_and_unknown_keys() {
    use mrd_signal_client::decode_authenticated_message;
    let fixture = fixture();
    let raw = format!(
        r#"{{"version":3,"message":{{"type":"session_intent_v3","payload":{}}}}}"#,
        fixture["intent_compact"].as_str().unwrap()
    );
    assert!(decode_authenticated_message(&raw).is_ok());
    for (name, altered) in [
        (
            "version",
            raw.replacen(r#""version":3"#, r#""version":999,"version":3"#, 1),
        ),
        (
            "message unknown",
            raw.replacen(
                r#""type":"session_intent_v3""#,
                r#""unknown":true,"type":"session_intent_v3""#,
                1,
            ),
        ),
        (
            "request codec",
            raw.replacen(r#""codec":"h264""#, r#""codec":"other","codec":"h264""#, 1),
        ),
        (
            "issuer counter",
            raw.replacen(r#""counter":2"#, r#""counter":999,"counter":2"#, 1),
        ),
    ] {
        assert_ne!(altered, raw, "mutation did not apply: {name}");
        assert!(
            decode_authenticated_message(&altered).is_err(),
            "accepted ambiguous browser {name}"
        );
    }
}
