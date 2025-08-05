pub struct SuiModuleResolver;

impl move_core_types::resolver::ModuleResolver for SuiModuleResolver {
    type Error = String;

    fn get_module(
        &self,
        _: &move_core_types::language_storage::ModuleId,
    ) -> Result<Option<Vec<u8>>, Self::Error> {
        Ok(None)
    }
}
