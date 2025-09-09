//! Macros for declaring Sui package layouts and generating typed argument indexers.
//!
//! These macros are used by integrations/presets to:
//! - Declare supported `modules` and `functions` for a package
//! - Generate typed index enums and getters for function arguments
//! - Produce a `Config` that backs `SuiIntegrationConfig` for `can_handle` checks
//!
//! Constraints and notes:
//! - Index getters decode only primitive numeric types and `bool` that implement `FromLeBytes`.
//!   They rely on `utils::decode_number`, which rejects `Object` arguments.
//! - The argument indices are positional in the `SuiProgrammableMoveCall.arguments` array.
//! - Package ids are stored as string keys via `stringify!(0x...)`. Use the exact on-chain id.
//! - If a function signature changes (order or types), the generated getters will return
//!   `VisualSignError` at runtime. Keep configs aligned with the on-chain ABI.
//! - These macros do not query chain state; they are purely declarative code generators.

#[macro_export]
macro_rules! __gen_module {
  (
    $module_name:ident as $ModVariant:ident => $FuncEnum:ident : {
      $(
        $fn_snake:ident as $FnVariant:ident => $IdxEnum:ident (
          $(
            $param_snake:ident
              as $ParamVariant:ident
              : $param_ty:ty
              => $param_idx:expr
                => $getter_name:ident
          ),* $(,)?
        )
      ),* $(,)?
    }
  ) => {
    /// Function variants for the `$module_name` module.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum $FuncEnum {
      $( $FnVariant ),*
    }

    impl TryFrom<&str> for $FuncEnum {
      type Error = visualsign::errors::VisualSignError;

      fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
          $( stringify!($fn_snake) => Ok(Self::$FnVariant), )*
          _ => Err(visualsign::errors::VisualSignError::DecodeError(format!("Unsupported function name: {value}"))),
        }
      }
    }

    impl $FuncEnum {
      /// Returns the list of supported function names for `$module_name`.
      pub fn get_supported_functions() -> Vec<&'static str> {
        vec![ $( stringify!($fn_snake) ),* ]
      }
    }


    $(
      /// Index enum for `$fn_snake` arguments and generated typed getters.
      #[derive(Hash, Eq, PartialEq, Debug, Clone, Copy)]
      pub enum $IdxEnum {
        $( $ParamVariant = $param_idx, )*
      }

      impl $IdxEnum {
        $(
          /// Getter for the `$param_snake` argument (position `$param_idx`) as `$param_ty`.
          ///
          /// Fails if the argument is missing, not `Pure`, or cannot be decoded as `$param_ty`.
          pub fn $getter_name (
            inputs: &[sui_json_rpc_types::SuiCallArg],
            args: &[sui_json_rpc_types::SuiArgument],
          ) -> Result<$param_ty, visualsign::errors::VisualSignError> {
            let idx = $crate::utils::get_index(
              args,
              Some($IdxEnum::$ParamVariant as usize),
            )? as usize;

            let arg = inputs
              .get(idx)
              .ok_or(visualsign::errors::VisualSignError::MissingData(
                concat!(stringify!($param_snake), " not found").into(),
              ))?;

            $crate::utils::decode_number::<$param_ty>(arg)
          }
        )*
      }
    )*
  };
}

/// Top-level macro for declaring a package layout and generating:
/// - Module/function enums and typed argument getters (via `__gen_module!`)
/// - A `Config` that implements `SuiIntegrationConfig`
/// - A process-global `OnceLock<Config>` for lazy initialization
///
/// Multiple packages can be defined in a single invocation. See existing configs
/// under `src/presets/*/config.rs` for examples.
#[macro_export]
macro_rules! chain_config {
  (
    // Configure the generated names for Config and static Lazy.
    config $static_name:ident as $struct_name:ident;

    $(
      $pkg_key:ident => {
        package_id => $pkg_id:expr,
        modules as $ModEnum:ident : {
          $(
            $mod_name:ident as $ModVariant:ident => $FuncEnum:ident : {
              $(
                $fn_snake:ident as $FnVariant:ident => $IdxEnum:ident (
                  $(
                    $param_snake:ident
                      as $ParamVariant:ident
                      : $param_ty:ty
                      => $param_idx:expr
                      => $getter_name:ident
                  ),* $(,)?
                )
              ),* $(,)?
            }
          ),* $(,)?
        }
      }
    ),* $(,)?
  ) => {
    $(
      /// Module variants for a declared package.
      #[derive(Debug, Clone, Copy, PartialEq, Eq)]
      pub enum $ModEnum {
        $( $ModVariant ),*
      }

        impl TryFrom<&str> for $ModEnum {
          type Error = visualsign::errors::VisualSignError;

          fn try_from(value: &str) -> Result<Self, Self::Error> {
            match value {
              $( stringify!($mod_name) => Ok(Self::$ModVariant), )*
              _ => Err(visualsign::errors::VisualSignError::DecodeError(format!("Unsupported module name: {value}"))),
            }
          }
        }
    )*

    // 1) Generate module-level code (enums + indexes + getters)
    $(
      $(
        $crate::__gen_module!(
          $mod_name as $ModVariant => $FuncEnum : {
            $(
              $fn_snake as $FnVariant => $IdxEnum (
                $(
                  $param_snake
                    as $ParamVariant
                    : $param_ty
                    => $param_idx
                    => $getter_name
                ),*
              )
            ),*
          }
        );
      )*
    )*

    // 2) Generate Config + impl + static Lazy
    /// Generated config type with precomputed package → module → function map.
    pub struct $struct_name {
      pub data: $crate::core::SuiIntegrationConfigData,
    }

    impl $crate::core::SuiIntegrationConfig for $struct_name {
      /// Builds the config map. Package ids are stored as string keys using
      /// `stringify!(0x...)`. Keep these in sync with on-chain deployments.
      fn new() -> Self {
        let mut packages = std::collections::HashMap::new();

        $(
          {
            let mut modules = std::collections::HashMap::new();

            $(
              modules.insert(
                stringify!($mod_name),
                $FuncEnum::get_supported_functions(),
              );
            )*

            // Store package id as a string key. We stringify the expression
            // (e.g., the 0x... literal) into "0x...".
            packages.insert(stringify!($pkg_id), modules);
          }
        )*

        Self {
          data: $crate::core::SuiIntegrationConfigData { packages },
        }
      }

      fn data(&self) -> &$crate::core::SuiIntegrationConfigData {
        &self.data
      }
    }

    /// Process-global lazy holder for the generated config.
    pub static $static_name: std::sync::OnceLock<$struct_name> = std::sync::OnceLock::new();
  };
}
