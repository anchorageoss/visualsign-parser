#[macro_export]
macro_rules! __gen_module {
  (
    $module_name:ident => $FuncEnum:ident : {
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
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum $FuncEnum {
      $( $FnVariant ),*
    }

    impl TryFrom<&str> for $FuncEnum {
      type Error = String;

      fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
          $( stringify!($fn_snake) => Ok($FuncEnum::$FnVariant), )*
          _ => Err(format!("Unsupported function name: {}", value)),
        }
      }
    }

    impl $FuncEnum {
      pub fn as_str(&self) -> &'static str {
        match self {
          $( $FuncEnum::$FnVariant => stringify!($fn_snake), )*
        }
      }

      pub fn get_supported_functions() -> Vec<&'static str> {
        vec![ $( stringify!($fn_snake) ),* ]
      }
    }

    impl AsRef<str> for $FuncEnum {
      fn as_ref(&self) -> &str {
        self.as_str()
      }
    }

    impl std::fmt::Display for $FuncEnum {
      fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
      ) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
      }
    }


    $(
      #[derive(Hash, Eq, PartialEq, Debug, Clone, Copy)]
      pub enum $IdxEnum {
        $( $ParamVariant = $param_idx, )*
      }

      impl $IdxEnum {
        $(
          pub fn $getter_name (
            inputs: &[sui_json_rpc_types::SuiCallArg],
            args: &[sui_json_rpc_types::SuiArgument],
          ) -> Result<$param_ty, visualsign::errors::VisualSignError> {
            $crate::utils::decode_number::<$param_ty>(
              inputs
                .get(
                  $crate::utils::get_index(
                    args,
                    Some($IdxEnum::$ParamVariant as usize),
                  )? as usize,
                )
                .ok_or(visualsign::errors::VisualSignError::MissingData(
                  concat!(stringify!($param_snake), " not found").into(),
                ))?,
            )
          }
        )*
      }
    )*
  };
}

// Top-level macro: generates module code, config struct + impl, and static
// Lazy. You can define multiple packages and modules in one go.
#[macro_export]
macro_rules! chain_config {
  (
    // Configure the generated names for Config and static Lazy.
    config $static_name:ident as $struct_name:ident;

    $(
      $pkg_key:ident => {
        package_id => $pkg_id:expr,
        modules: {
          $(
            $mod_name:ident => $FuncEnum:ident : {
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
    // 1) Generate module-level code (enums + indexes + getters)
    $(
      $(
        $crate::__gen_module!(
          $mod_name => $FuncEnum : {
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
    pub struct $struct_name {
      pub data: $crate::core::SuiIntegrationConfigData,
    }

    impl $crate::core::SuiIntegrationConfig for $struct_name {
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

    pub static $static_name: std::sync::OnceLock<$struct_name> = std::sync::OnceLock::new();
  };
}
