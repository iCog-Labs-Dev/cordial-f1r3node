use k256::ecdsa::SigningKey;

fn main() {
    let validators = [
        ("CORDIAL_FOUR_NODE_1_PRIVATE_KEY", [0x01_u8; 32]),
        ("CORDIAL_FOUR_NODE_2_PRIVATE_KEY", [0x02_u8; 32]),
        ("CORDIAL_FOUR_NODE_3_PRIVATE_KEY", [0x03_u8; 32]),
        ("CORDIAL_FOUR_NODE_4_PRIVATE_KEY", [0x04_u8; 32]),
    ];

    println!("# Four-node demo validator keys");
    println!("# Environment entries");
    for (name, private_key) in validators {
        let signing_key = SigningKey::from_slice(&private_key).expect("valid demo private key");
        let public_key = signing_key.verifying_key().to_encoded_point(false);
        println!("{name}={}", hex::encode(private_key));
        println!(
            "{}={}",
            name.replace("PRIVATE", "PUBLIC"),
            hex::encode(public_key.as_bytes())
        );
    }

    println!();
    println!("# bonds.txt entries");
    for (_, private_key) in validators {
        let signing_key = SigningKey::from_slice(&private_key).expect("valid demo private key");
        let public_key = signing_key.verifying_key().to_encoded_point(false);
        println!("{} 1", hex::encode(public_key.as_bytes()));
    }
}
