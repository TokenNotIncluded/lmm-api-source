use super::*;

const PROOF: &str = "synthetic-edge-proof-for-private-uds-1111111111";

fn metadata() -> MetadataMap {
    let mut m = MetadataMap::new();
    m.insert(crate::edge_access::PROOF_HEADER, PROOF.parse().unwrap());
    m.insert(
        crate::edge_access::CLIENT_HEADER,
        "::ffff:203.0.113.7".parse().unwrap(),
    );
    m
}

#[test]
fn assistant_edge_requires_same_private_proof_and_one_canonical_address() {
    assert_eq!(
        verified_edge_peer(Some(PROOF.as_bytes()), &metadata()).unwrap(),
        Some("203.0.113.7".parse().unwrap())
    );
    assert!(verified_edge_peer(None, &metadata()).is_err());
    assert!(verified_edge_peer(Some(b"wrong-proof"), &metadata()).is_err());
    assert_eq!(verified_edge_peer(None, &MetadataMap::new()).unwrap(), None);
    for key in [
        crate::edge_access::PROOF_HEADER,
        crate::edge_access::CLIENT_HEADER,
    ] {
        let mut m = metadata();
        m.remove(key);
        assert!(verified_edge_peer(Some(PROOF.as_bytes()), &m).is_err());
        let mut m = metadata();
        m.append(key, m.get(key).unwrap().clone());
        assert!(verified_edge_peer(Some(PROOF.as_bytes()), &m).is_err());
    }
    for ip in [
        "203.0.113.7,127.0.0.1",
        "203.0.113.7:80",
        "fe80::1%lo",
        " 203.0.113.7",
        "203.0.113.7 ",
    ] {
        let mut m = metadata();
        m.insert(crate::edge_access::CLIENT_HEADER, ip.parse().unwrap());
        assert!(
            verified_edge_peer(Some(PROOF.as_bytes()), &m).is_err(),
            "{ip}"
        );
    }
    let mut m = MetadataMap::new();
    m.insert("x-forwarded-for", "203.0.113.7".parse().unwrap());
    m.insert("x-real-ip", "203.0.113.7".parse().unwrap());
    assert_eq!(
        verified_edge_peer(Some(PROOF.as_bytes()), &m).unwrap(),
        None
    );
}
