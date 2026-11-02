// Copyright (C) 2026 LIghtJUNction. AGPL-3.0-or-later.
use super::cny_to_usd;

// Go shopspring/decimal v1.4.0 oracle, math/rand seed 20261101. Each tuple
// stores IEEE-754 input, explicit positive rate, and legacy decimal division
// output bits; this bounded sample does not supply historical exchange rates.
#[test]
fn cny_conversion_matches_legacy_decimal_oracle_sample() {
    let cases: &[(u64, u64, u64)] = &[
        (0x406e4d4c58d6a559, 0x3f1388ae10f3da1a, 0x4148d1ea2645b2a7),
        (0x3e896a005e88eb3d, 0x4124e95ecc8b105f, 0x3d537160d8cde169),
        (0xbdffb77cdb995daf, 0x3e97353a69b31719, 0xbf55ddbc320eec9d),
        (0xc00acdd53177d134, 0x4050e91045440f69, 0xbfa95c69a4d37bdc),
        (0x3e366bd456b6a9c5, 0x40604493be2838fb, 0x3dc60d51847dbaed),
        (0x3f13d70cb79d52db, 0x40172adae678a6fe, 0x3eeb67715fa1e0b3),
        (0x3fd01ecb1a144814, 0x40bafdd5f38c10f5, 0x3f031c8bd9bf56a8),
        (0xbf65f6a30433d09c, 0x40f765a0ce34bf91, 0xbe5e0a1119055484),
        (0xc16663920ab55faf, 0x3f370e7298283d7f, 0xc21f12d6fcea1b8a),
        (0x40783ea13d3240e2, 0x41213b8bca959f19, 0x3f4682afae234719),
        (0xc08605b67e9a50bd, 0x3f11b7af21f090f8, 0xc163e33307538e95),
        (0x3f1861536fdd8836, 0x3fa23b22358effd0, 0x3f65658d85d88b88),
        (0x3e0b09eb1bd123aa, 0x40ed7ae03224f4f2, 0x3d0d45fd6237ebe3),
        (0x41619ec1bef3e2a2, 0x40f378aec8832514, 0x405cf5223cf678f5),
        (0x3fed4baff1526bee, 0x408f8883547f6547, 0x3f4dbab20a2f5533),
        (0x3f14bb63d6f48695, 0x3f837f6c3c5f787b, 0x3f810348fda8e8d0),
        (0xc0536ca1090a9e95, 0x3ef0d4fade859138, 0xc15276d8ce92c668),
        (0xbfdac49f6bbb131a, 0x40acb2b0d8e70e8c, 0xbf1dd915a38674fc),
        (0xbf427cee703879f9, 0x40c27610705547e4, 0xbe7005f3a0c4a0c3),
        (0x40686168738ef93b, 0x408c4db57dbb46b2, 0x3fcb9089833ad560),
        (0xc1b7febb691fa3bc, 0x3eb0e9ef04f6fa6b, 0xc2f6b2dbb303e114),
        (0x3fb2573692990f92, 0x40e8f70218292913, 0x3eb7825414ec7a57),
        (0xbeb755376e10123b, 0x3f0d0b3a66a1ce6a, 0xbf99b52e9999b622),
        (0x3f5c55a417867d33, 0x4070f3bd8bd906b9, 0x3edabe3df9509133),
        (0x3fd7fcfa79a7a480, 0x3fb790ce21d9aabe, 0x40104971d6fbdc9e),
        (0xbf7d2b0f839202e3, 0x401b1a6655b4afaf, 0xbf513816a8dbb184),
        (0xc0de78a9c27c1639, 0x3f7bf583c377dcba, 0xc151700d40a6b92c),
        (0x408881ba03a348e8, 0x3fb99d8c88f913b5, 0x40be9d6f8af36779),
        (0x3fdf007fa3d3e518, 0x40b57b7a4e15e29c, 0x3f171716966fffbe),
        (0xbedb60e666372dbf, 0x410e4a5cf3866ee5, 0xbdbcec73dcedb879),
        (0xc19019a9888f155f, 0x3f955b486b15f8d8, 0xc1e81fc3f87b0d2b),
        (0x408cf86d4e34cfad, 0x40185f0922099f5a, 0x406304f89f2beb39),
    ];
    for &(amount, rate, expected) in cases {
        let actual = cny_to_usd(f64::from_bits(amount), f64::from_bits(rate));
        assert_eq!(
            actual,
            Some(f64::from_bits(expected)),
            "amount={amount:x} rate={rate:x}"
        );
    }
}
