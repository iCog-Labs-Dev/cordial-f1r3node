use cordial_miners_core::NodeId;
use cordial_por::{RATING_SIGNING_DOMAIN, RatingRecord, canonical_rating_payload};

fn rating() -> RatingRecord {
    let mut rating = RatingRecord::new(
        0x0102_0304_0506_0708,
        NodeId(vec![0xaa, 0xbb]),
        NodeId(vec![0xcc]),
        0x1112_1314_1516_1718,
        vec![0xde, 0xad],
    );
    rating.interaction_ref = Some(vec![0x21, 0x22, 0x23]);
    rating
}

#[test]
fn canonical_payload_has_the_documented_v1_layout() {
    let mut expected = RATING_SIGNING_DOMAIN.to_vec();
    expected.extend_from_slice(&0x0102_0304_0506_0708_u64.to_be_bytes());
    expected.extend_from_slice(&2_u64.to_be_bytes());
    expected.extend_from_slice(&[0xaa, 0xbb]);
    expected.extend_from_slice(&1_u64.to_be_bytes());
    expected.push(0xcc);
    expected.extend_from_slice(&0x1112_1314_1516_1718_u64.to_be_bytes());
    expected.push(1);
    expected.extend_from_slice(&3_u64.to_be_bytes());
    expected.extend_from_slice(&[0x21, 0x22, 0x23]);

    assert_eq!(canonical_rating_payload(&rating()), expected);
}

#[test]
fn signature_bytes_are_not_part_of_the_signed_payload() {
    let first = rating();
    let mut second = first.clone();
    second.signature = vec![1, 2, 3, 4];

    assert_eq!(
        canonical_rating_payload(&first),
        canonical_rating_payload(&second)
    );
}

#[test]
fn absent_interaction_reference_has_a_distinct_encoding() {
    let mut without_reference = rating();
    without_reference.interaction_ref = None;
    let mut empty_reference = rating();
    empty_reference.interaction_ref = Some(Vec::new());

    assert_ne!(
        canonical_rating_payload(&without_reference),
        canonical_rating_payload(&empty_reference)
    );
    assert_eq!(
        canonical_rating_payload(&without_reference).last(),
        Some(&0)
    );
}
