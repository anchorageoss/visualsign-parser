mod core;
mod integrations;
mod presets;
pub mod utils;

pub use core::*;
pub use utils::*;

#[cfg(test)]
mod tests {
    use super::*;
    use visualsign::vsptrait::{Transaction, VisualSignConverter, VisualSignOptions};

    #[test]
    fn test_solana_charset_validation() {
        // Test that Solana parser produces ASCII-only output for various transaction types
        let test_cases = vec![(
            "Jupiter swap (previously had Unicode arrow)",
            "AQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABAAkSTXq/T5ciKTTbZJhKN+HNd2Q3/i8mDBxbxpek3krZ664CMz4dTWd4gwDq6aKU/sqHgTzleVA7bTCOy59kSOO+0EPkGS7bWuT/2yiCuaADtj/v6d+KwyTj46OQM2MjIq6hTqzVdwLTW8t+UsWMrwHEvc/r814OmVR9yLVQZujbWvpTh0XSNlF7uoIvuHyKD/16mBElrNa/eT8vB1KVUaN8IoaTvZbN4b7iiv8Q8cl5bDecNqCXzTS1Xmsmh5b2UVZniTbtX0AYG5QKiSDC10m0caM6frmEVukpjEWOk7F/0OzFKL0A0HdMWTIMuQj4xBuP3csLyGzVO/MXtPu6woNViO2O9ocxd1YSDcIwhrzHY3a9ewvycRH5q662TcQqdxD6AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAEedVb8jHAbu50xW7OaBUH/bGy3qP0jlECsc2iVrwTjwabiFf+q4GE+2h/Y0YYwDXaxDncGus7VZig8AAAAAABBt324ddloZPZy+FGzut5rBy0he1fWzeROoz1hX7/AKkOA2hfjpCQU+RYEhxm9adq7cdwaqEcgviqlSqPK3h5qVJNNVq4xx0JIWWE9kFLvpQK5lvS5UCde3W3QfWYLIxYjJclj04kifG7PRApFI4NgwtaE5na/xCEBI572Nvp+Fm0P/on9df2SnTAmx8pWHneSwmrNt/J3VFLMhqns4zl6Mb6evO+2606PWXzaqvJdDGxu+TC0vbg5HymAgNFL11hXuFhKBWRymmouYdcNxL6PjM1Bkcio0R+AtqA/P3C3jAFDwYABgALCQwBAQkCAAYMAgAAAEBCDwAAAAAADAEGAREKFQwABgUKEQoQCg0MAAQGAwUHCAECDiTlF8uXeuOtKgEAAAARAWQAAUBCDwAAAAAAtEADAAAAAAAyAAAMAwYAAAEJ",
        )];

        for (description, tx_data) in test_cases {
            // Parse the transaction
            let transaction_wrapper = SolanaTransactionWrapper::from_string(tx_data)
                .unwrap_or_else(|e| panic!("Failed to parse {description}: {e:?}"));

            // Convert to VisualSign payload
            let payload = SolanaVisualSignConverter
                .to_visual_sign_payload(
                    transaction_wrapper,
                    VisualSignOptions {
                        decode_transfers: true,
                        transaction_name: Some(description.to_string()),
                    },
                )
                .unwrap_or_else(|e| panic!("Failed to convert {description} to payload: {e:?}"));

            // Test charset validation
            let validation_result = payload.validate_charset();
            assert!(
                validation_result.is_ok(),
                "{} should produce ASCII-only output, got validation error: {:?}",
                description,
                validation_result.err()
            );

            // Test that to_validated_json works
            let json_result = payload.to_validated_json();
            assert!(
                json_result.is_ok(),
                "{} output should serialize with charset validation, got error: {:?}",
                description,
                json_result.err()
            );

            let json_string = json_result.unwrap();

            // Verify no unicode escapes are present
            assert!(
                !json_string.contains("\\u"),
                "{description} JSON should not contain unicode escapes"
            );

            // Verify the JSON is valid ASCII - this catches ALL non-ASCII characters
            assert!(
                json_string.is_ascii(),
                "{description} JSON output should be ASCII only"
            );

            tracing::info!("✅ {} passed charset validation", description);
        }
    }

    #[test]
    fn test_solana_decode_no_unicode_escapes() {
        let transaction_data = "AQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABABQjTXq/T5ciKTTbZJhKN+HNd2Q3/i8mDBxbxpek3krZ664l11NTYMPrFRIX4Lz5B1rawnwF8fo+zTq+LrHZd2V/Si9+/8BZYNrmZWg1MDFx7olmFC1qg/HqKxDZNRTsw6UVT9rPU1lVeStbG3jHrBDk4J55uG9w2rkLAR10R2ENmBJpdHxO6T7Szv3Sd/D8gcUfdPCSOUaog8zuYAfTraWosHcEejgcORU496O6Qrr+hB1FPybVLnGmZEP2rx7ddIr9d0Ilb6c5G3LW+bBCMASFsppoSCdyVWmODJrwjNYXKa13/l5fW019HV90w1MnMFDHg1VOVUhn2fLkSo61U182yok27V9AGBuUCokgwtdJtHGjOn65hFbpKYxFjpOxf9DskG5Y9TKgvjyWMtSTofnmutDxUhf8mAIvlIRkxkC9HcfGFmdldg4HGS4H7wtXkv2nhMukBo0X2VXHcYssdUx4h9e3/xKVXXP7+7SjH9l1cDmHU2qpfbNr/cqcliUA5EIA2NP9PjUA/HraW3rBfN1vIglfC/5kTTcOSi6JnSyhs7Xk/3/MhpKJ95PxNGMvow4vaWo/h7CvB1sVLalbq1Qai+nUSIsH/jmbGpFV5YIbaX1DAWwKPE87vKKvtB0BYzBXAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADBkZv5SEXMv/srbpyw5vnvIzlu8X3EmssQ5s6QAAAAAR51VvyMcBu7nTFbs5oFQf9sbLeo/SOUQKxzaJWvBOPBpuIV/6rgYT7aH9jRhjANdrEOdwa6ztVmKDwAAAAAAEG3fbh12Whk9nL4UbO63msHLSF7V9bN5E6jPWFfv8AqQbd9uHudY/eGEJdvORszdq2GvxNg7kNJ/69+SjYoYv8DQlccSBT/RtPaPA3gX+BO58I+Vw58YIIMGa/gNL3yosNDMH4743ur5BEniN/ODyXbja9U2UtNnngEY0Vi3MeQg3H0AP9nsTh3WPvZEBDc/DoMc0tKISU2r7TeHXsu5vcHM6YmDVt6z8sNI3KokBPVY6Q7DXK4znaxlUELWQDV69j2ZF0gyVfuUtC/HsgXvGDI3DNIBOa3EktVdLIi1IYiWqudc1LV+DIFppHpHG0HuUes6bWvPNcB2gSxOmiz9XojJclj04kifG7PRApFI4NgwtaE5na/xCEBI572Nvp+FmsGuPQh/KSNwYlSPcMTASuwqmVaUmG58u0Z1IGIdOGMKzV0ebZPU1hD7s1KzR+EKUhApOszmbDYfAhVeUESa5jtD/6J/XX9kp0wJsfKVh53ksJqzbfyd1RSzIap7OM5ejG1lsEBH3TZF6pmqj6moKw9Y+3xGMHqFo986DDMOvINsb6evO+2606PWXzaqvJdDGxu+TC0vbg5HymAgNFL11hzgEOYK/tsicXvWMZL1QUWj+WWjO7gtLHAp6yzh4ggmT1G5t56F3oMXO9/md9Dan95RdKnFZ/h5iqL/+hVtYYxAu1xm9ykXCakK7v8nmlE13mddTZFM3G9sM0vxzQnq3oCBAABQLAXBUAEAAJAwQXAQAAAAAAGwYACAASDxMBAQ8CAAgMAgAAAEBCDwAAAAAAEwEIAREbBgAGACEPEwEBESsTHAAIBQ0GEiERER4RGCIEHAUOAwoMCxMfHx0WHCAhDg0JBwIBGhUZFxMUKMEgmzNB1pyBBQIAAAAZZAABT2QBAkBCDwAAAAAApV4DAAAAAAAyAAATAwgAAAEJ";

        // Parse the transaction
        let transaction_wrapper = SolanaTransactionWrapper::from_string(transaction_data)
            .expect("Should parse transaction successfully");

        // Convert to VisualSign payload with transfer decoding enabled
        let payload = SolanaVisualSignConverter
            .to_visual_sign_payload(
                transaction_wrapper,
                VisualSignOptions {
                    decode_transfers: true,
                    transaction_name: Some("Unicode Escape Test".to_string()),
                },
            )
            .expect("Should convert to payload successfully");

        // Convert to JSON
        let json_result = payload
            .to_validated_json()
            .expect("Should serialize to JSON with valid charset");

        tracing::info!("✅ Transaction decoded successfully without unicode escapes");
        tracing::info!("✅ Transaction type: {}", payload.payload_type);
        tracing::info!("✅ Number of fields: {}", payload.fields.len());
        tracing::info!("📄 Emitted JSON for visual inspection:");
        tracing::info!("{}", json_result);
    }
}
