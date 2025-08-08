mod config;

use config::{NATIVE_STAKING_CONFIG, SuiSystemFunctions};

use crate::core::{CommandVisualizer, SuiIntegrationConfig, VisualizerContext};
use crate::utils::{create_address_field, get_index};

use move_core_types::runtime_value::MoveValue;
use sui_json::{MoveTypeLayout, SuiJsonValue};
use sui_json_rpc_types::{SuiArgument, SuiCallArg, SuiCommand};
use sui_types::base_types::SuiAddress;

use visualsign::{
    SignablePayloadField, SignablePayloadFieldCommon, SignablePayloadFieldListLayout,
    field_builders::create_amount_field,
};

pub struct SuiNativeStakingVisualizer;

impl CommandVisualizer for SuiNativeStakingVisualizer {
    fn visualize_tx_commands(&self, context: &VisualizerContext) -> Option<SignablePayloadField> {
        let Some(SuiCommand::MoveCall(pwc)) = context.commands().get(context.command_index())
        else {
            return None;
        };

        let function = match pwc.function.as_str().try_into() {
            Ok(function) => function,
            Err(_) => return None,
        };

        match function {
            SuiSystemFunctions::AddStake => {
                let amount = get_stake_amount(context.commands(), context.inputs(), &pwc.arguments)
                    .unwrap_or_default();
                let receiver =
                    get_stake_receiver(context.inputs(), &pwc.arguments).unwrap_or_default();

                Some(SignablePayloadField::ListLayout {
                    common: SignablePayloadFieldCommon {
                        fallback_text: "Stake Command".to_string(),
                        label: "Stake Command".to_string(),
                    },
                    list_layout: SignablePayloadFieldListLayout {
                        fields: vec![
                            create_address_field(
                                "From",
                                &context.sender().to_string(),
                                None,
                                None,
                                None,
                                None,
                            ),
                            create_address_field(
                                "Validator",
                                &receiver.to_string(),
                                None,
                                None,
                                None,
                                None,
                            ),
                            create_amount_field("Amount", &amount.to_string(), "MIST"),
                        ],
                    },
                })
            }
            SuiSystemFunctions::WithdrawStake => Some(SignablePayloadField::ListLayout {
                common: SignablePayloadFieldCommon {
                    fallback_text: "Withdraw Command".to_string(),
                    label: "Withdraw Command".to_string(),
                },
                list_layout: SignablePayloadFieldListLayout {
                    fields: vec![create_address_field(
                        "From",
                        &context.sender().to_string(),
                        None,
                        None,
                        None,
                        None,
                    )],
                },
            }),
        }
    }

    fn get_config(&self) -> Option<&dyn SuiIntegrationConfig> {
        Some(&*NATIVE_STAKING_CONFIG)
    }
}

fn get_stake_receiver(inputs: &[SuiCallArg], args: &[SuiArgument]) -> Option<SuiAddress> {
    let receiver_input = inputs.get(get_index(args, Some(args.len() - 1))? as usize)?;

    receiver_input.pure()?.to_sui_address().ok()
}

fn get_stake_amount(
    commands: &[SuiCommand],
    inputs: &[SuiCallArg],
    args: &[SuiArgument],
) -> Option<u64> {
    let result_command = commands.get(get_index(args, Some(1))? as usize)?;

    match result_command {
        SuiCommand::SplitCoins(_, input_coin_args) => {
            let amount_arg = inputs.get(get_index(input_coin_args, Some(0))? as usize)?;
            let Ok(MoveValue::U64(decoded_value)) = SuiJsonValue::to_move_value(
                &amount_arg.pure()?.to_json_value(),
                &MoveTypeLayout::U64,
            ) else {
                return None;
            };
            Some(decoded_value)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::transaction_string_to_visual_sign;
    use visualsign::vsptrait::VisualSignOptions;

    #[test]
    fn test_stake_commands() {
        // https://suivision.xyz/txblock/4cccJLKehRtyRQY7TaNUJiM4ipauWCn8S3GNJr9RtfCN
        let test_data = "AQAAAAAAAwAIAGKs63UDAAABAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAFAQAAAAAAAAABACAArnjT5bpda43jJFVHT1KBG5VhfLrTnr9Pni2vZxh0BwICAAEBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAMKc3VpX3N5c3RlbRFyZXF1ZXN0X2FkZF9zdGFrZQADAQEAAgAAAQIAchml1wtdzMahHtnC+vK/PAN3Y1Nua3n0b+llLNlP63sS91480t7crkx10tMf1GBphnFn9ImRSCkSz+/vgVnXpCH+wrgjAAAAACA0F4UpabC9/7RFUiBnEiOjfQUh7WwycuwxC4HXNWCB87xhtd+38zkA5oow9A8dNJZLJLmExMhHZtVr2Z54J5dCM8+6IwAAAAAgVo0BnK/9uyVcuP4Dh6Zz/AoGPRcuforA522PgiEMj+ExGC1sSX2Iz5VaSZKDG0S4hUquzd+gIG6HrubmTB4+H2xQvCMAAAAAIMZWzEKhYGfx/BBVEOwj0BPKog1L9vsjFOMVGz+Ccz/1UjA6TZRCYu97v9k62s814RDTXBDCysramrxWkw8rC4WG1rojAAAAACC5twStwiG1CYMchoX6fuLsxbpZflZqa/Nfqgor4F2FZD+WWCYBIOd63H/RJp8L1dGzXJ1a2ccCShJ+PDrr52JQ4je7IwAAAAAg7IkUrK8NWz3Eqvt/v5sge65N6ulWG3jZxCTcK7qRbWUL/tH0Ysraua6BptIBZqYGaxV6xC9vWMfTe+Ip5jE+I7ZevCMAAAAAIGAAQtBVw7aOXRphh8b9pv3jgnyzT/YC574vRTCI9OQilIwD9rHfpNGU2fTQS6FUiyT02WUBUSJwU89ZEeWB8sh8ULsjAAAAACDofQBTuJq5tRuROvF8G+iXBf97nefwvk7EABk3ozFDv3KMQb/vp6PKjBZPNJAWeGNGlwQXmLmssjlgiaetA+5XRmK7IwAAAAAgIV9blUqSik4sllVwRF2L+ubVGWFHQhtmNFBZpuwBd2bKy8PFJe+VJiA++e9bXK/fjvCK0RpZ7VprD2eEwy3ODYi/uyMAAAAAIMvIs1NC8//tjFVBz5SbJj9qqLh2qbF1RfNZW0wx5Mo6X9Lx7+LuoE25ZFW5oSw44lmJ2vPae4KQ0R1kTfbRGiave7sjAAAAACCzmP55RKlPOqGJBdfS6eY+UjmlpTSGvTHP8hUWk7T4OYEUDI6TTxeUK1AnF+Xhiklt9fcZXZ1PVWiEiNq/u0Utz367IwAAAAAgZ42PQNaZfltc5MVc9Ja6ZzBJDrXsdgINGVW76jVNbyn+XOhRQaock9U7J1O371bGZeEAoriHNfGn3CkXGDnwX0GAuyMAAAAAIN7MiSc0QEvu9npIm1Prv2ORlUh992gEVMXByCyltfE/Coezo8orpYDdndeF2vFkJ/+vhmHQGWvxEyYkwnqcHzQ/jrsjAAAAACC/BIZAoP2mo+07tcbjR+dPEmQCZdGr/tU/LE/Pr+uap2LuRhUG8chU5FnphmyErbq6yYw3AlBGynionKP1QlgD0pK7IwAAAAAgSCHwEJRXpc21CWcbjZ1zC6seZmFxLA1/2ox1kg/3NNwjh8ocklBDNJQ0p018bGQnQ1/fmbQ3PASM6321c8Q49XCuuyMAAAAAIPuRIPYEeaHC3ghIxae9SYvjlctN+ICS/+f264nO4GHm8qdjD3lvHnR5iRAhWQ2grQ0fhVTojNHw4gzZfrjBkj0fgLwjAAAAACBWkHgrTPBmmqWNSjcdrfkH9/WSO7dGCgObuL+Z4XdhcXkbWK1fLyah0wbPUVlQKnJ04TEMb/pJ5VZQX3JUGT96alK8IwAAAAAgN0wfiUZurekECwSJYJTnNzs5zQOXSVbwxUOBZuZe13Xjle13WuEg8ZzCrsUDk9vveQAEPGoX5ilfN0bUCxE+YOw4vCMAAAAAIEiOQkW7xn/ypzTHbgEBr+2ria56PZNqDNGxoSlqcAqCchml1wtdzMahHtnC+vK/PAN3Y1Nua3n0b+llLNlP63shAgAAAAAAANChEAAAAAAAAAFhAAMXK+XvLV700RIKRRVecODdz7ix6ld6Xd7n3OA4FNQF9dctGN8cnisaVnkxhpmWExq9udXFE5taXf+6oPYdOwvQTyj2+JV1sMgV1T5PRxv9WG+kbKk5wGHh3oKpRtlEUw==";

        let payload = transaction_string_to_visual_sign(
            test_data,
            VisualSignOptions {
                decode_transfers: true,
                transaction_name: None,
            },
        )
        .expect("Failed to visualize tx commands");

        let _ = payload
            .fields
            .iter()
            .find(|f| f.label() == "Stake Command")
            .expect("Should have a Stake Command field");
    }

    #[test]
    fn test_withdraw_commands() {
        // https://suivision.xyz/txblock/4cccJLKehRtyRQY7TaNUJiM4ipauWCn8S3GNJr9RtfCN
        let test_data = "AQAAAAAAAgEBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAUBAAAAAAAAAAEBADtuZRRZcXabYn2eLpOPGq3onyss/0Kyuv3BoB3PQPiIJHpFHQAAAAAgDlI1Bti2mpZBb/rDxYkyB+lyANUGRTtYgKbRoBow53cBAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADCnN1aV9zeXN0ZW0WcmVxdWVzdF93aXRoZHJhd19zdGFrZQACAQAAAQEAPmYGcGNxVi5pj8Tk1ufHEB6SYs6TFjQYj+JG7623BnUCN8ccpwVmcafDNOXvnEAo6kzltjdniobA56to42fHdUio9wcjAAAAACDQVC4fMhsmX6OlHpAhyPR8LaRzgu43Bj8xrhlRY6YKG/Yv6m2ncHpPhbrEkOrSiyh1ID3T4FARE+raMUofCsQPqPcHIwAAAAAg5qp+jjoniUXPNG4N0/9XDFSpoUt0isbEUMiXjNtGivA+ZgZwY3FWLmmPxOTW58cQHpJizpMWNBiP4kbvrbcGdSECAAAAAAAADAqcAAAAAAAAAWEAkj0EN51BkbIUE/6lMi967MHGsBMl2i8TtntUnFhlC2rK8AW2fGQxc8mg1gTbV+2eHs1CsZ9m67cU4CWzA+9PAg//ECUrmzUzzsg0xYRgwDQDy9lAF8e6bpAa8/5Yec6s";

        let payload = transaction_string_to_visual_sign(
            test_data,
            VisualSignOptions {
                decode_transfers: true,
                transaction_name: None,
            },
        )
        .expect("Failed to visualize tx commands");

        let _ = payload
            .fields
            .iter()
            .find(|f| f.label() == "Withdraw Command")
            .expect("Should have a Withdraw Command field");
    }
}
