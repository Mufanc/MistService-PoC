use rsbinder::hub::ServiceManager;
use rsbinder::{SIBinder, Status};
use std::sync::Arc;

pub trait AddServiceEx {
    fn add_service(
        &self,
        name: &str,
        binder: SIBinder,
        allow_isolated: bool,
        dump_priority: i32,
    ) -> Result<(), Status>;
}

impl AddServiceEx for Arc<ServiceManager> {
    fn add_service(
        &self,
        name: &str,
        binder: SIBinder,
        allow_isolated: bool,
        dump_priority: i32,
    ) -> Result<(), Status> {
        macro_rules! impl_versions {
            ($($version: literal),+) => {
                paste::paste! {
                    match self.as_ref() {
                        $(ServiceManager::[< Android $version >](manager) => {
                            rsbinder::hub::[< android_ $version >]::IServiceManager::addService(
                                manager, name, &binder, allow_isolated, dump_priority
                            )
                        }),+
                    }
                }
            };
        }

        impl_versions!(13, 14, 16)
    }
}
