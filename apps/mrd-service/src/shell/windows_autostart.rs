//! Autostart configuration of the installed Windows background service.

use std::sync::Arc;

use anyhow::{Context, Result};
use windows::{
    core::PCWSTR,
    Win32::System::Services::{
        ChangeServiceConfigW, ENUM_SERVICE_TYPE, SC_HANDLE, SERVICE_ERROR, SERVICE_NO_CHANGE,
        SERVICE_START_TYPE,
    },
};
use windows_service::{
    service::{Service, ServiceAccess, ServiceStartType},
    service_manager::{ServiceManager, ServiceManagerAccess},
};

use super::AutostartPort;
use crate::windows_service::MRD_WINDOWS_SERVICE_NAME;

trait ServiceStartupPort: Send + Sync {
    fn query_start_type(&self, service_name: &str) -> Result<Option<ServiceStartType>>;
    fn set_start_type(&self, service_name: &str, start_type: ServiceStartType) -> Result<()>;
}

struct ScmServiceStartup;

impl ScmServiceStartup {
    fn open(service_name: &str, access: ServiceAccess) -> Result<Option<Service>> {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .context("Cannot connect to the Windows service manager")?;
        match manager.open_service(service_name, access) {
            Ok(service) => Ok(Some(service)),
            Err(windows_service::Error::Winapi(error)) if error.raw_os_error() == Some(1060) => {
                Ok(None)
            }
            Err(error) => Err(error).with_context(|| format!("Cannot open service {service_name}")),
        }
    }
}

impl ServiceStartupPort for ScmServiceStartup {
    fn query_start_type(&self, service_name: &str) -> Result<Option<ServiceStartType>> {
        Self::open(service_name, ServiceAccess::QUERY_CONFIG)?
            .map(|service| {
                service
                    .query_config()
                    .map(|config| config.start_type)
                    .with_context(|| format!("Cannot read startup configuration of {service_name}"))
            })
            .transpose()
    }

    fn set_start_type(&self, service_name: &str, start_type: ServiceStartType) -> Result<()> {
        let service = Self::open(service_name, ServiceAccess::CHANGE_CONFIG)?
            .with_context(|| format!("Windows service {service_name} is not installed"))?;
        // The windows-service change_config API also rewrites the command line,
        // account and dependencies. Null strings and SERVICE_NO_CHANGE preserve
        // every existing setting except the requested startup type.
        unsafe {
            ChangeServiceConfigW(
                SC_HANDLE(service.raw_handle()),
                ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
                SERVICE_START_TYPE(start_type.to_raw()),
                SERVICE_ERROR(SERVICE_NO_CHANGE),
                PCWSTR::null(),
                PCWSTR::null(),
                None,
                PCWSTR::null(),
                PCWSTR::null(),
                PCWSTR::null(),
                PCWSTR::null(),
            )
        }
        .with_context(|| format!("Cannot change startup configuration of {service_name}"))
    }
}

/// Reads and changes the startup type of the installed SCM service.
pub struct WindowsAutostart {
    service_name: String,
    startup: Arc<dyn ServiceStartupPort>,
}

impl WindowsAutostart {
    pub fn new(service_name: impl Into<String>) -> Self {
        let service_name = service_name.into();
        // Existing IPC constructors use the binary name; SCM uses the installer name.
        let service_name = if service_name == "mrd-service" {
            MRD_WINDOWS_SERVICE_NAME.to_string()
        } else {
            service_name
        };
        Self {
            service_name,
            startup: Arc::new(ScmServiceStartup),
        }
    }
}

impl AutostartPort for WindowsAutostart {
    fn is_enabled(&self) -> Result<bool> {
        let start_type = self
            .startup
            .query_start_type(&self.service_name)?
            .with_context(|| format!("Windows service {} is not installed", self.service_name))?;
        Ok(start_type == ServiceStartType::AutoStart)
    }

    fn set_enabled(&self, enabled: bool) -> Result<()> {
        self.startup.set_start_type(
            &self.service_name,
            if enabled {
                ServiceStartType::AutoStart
            } else {
                // Keep explicit user starts possible without starting at boot.
                ServiceStartType::OnDemand
            },
        )
    }

    fn is_supported(&self) -> bool {
        // Only a missing service means unsupported. Query errors must be exposed
        // by is_enabled rather than disguised as an unsupported platform.
        !matches!(self.startup.query_start_type(&self.service_name), Ok(None))
    }

    fn get_entry_name(&self) -> &str {
        &self.service_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct TestStartup {
        start_type: Mutex<Option<ServiceStartType>>,
        query_fails: bool,
        change_fails: bool,
    }

    impl ServiceStartupPort for TestStartup {
        fn query_start_type(&self, service_name: &str) -> Result<Option<ServiceStartType>> {
            assert_eq!(service_name, MRD_WINDOWS_SERVICE_NAME);
            anyhow::ensure!(!self.query_fails, "query denied");
            Ok(*self.start_type.lock().unwrap())
        }

        fn set_start_type(&self, service_name: &str, start_type: ServiceStartType) -> Result<()> {
            assert_eq!(service_name, MRD_WINDOWS_SERVICE_NAME);
            anyhow::ensure!(!self.change_fails, "change denied");
            *self.start_type.lock().unwrap() = Some(start_type);
            Ok(())
        }
    }

    fn port(startup: Arc<TestStartup>) -> WindowsAutostart {
        WindowsAutostart {
            service_name: MRD_WINDOWS_SERVICE_NAME.to_string(),
            startup,
        }
    }

    fn startup(start_type: Option<ServiceStartType>) -> Arc<TestStartup> {
        Arc::new(TestStartup {
            start_type: Mutex::new(start_type),
            query_fails: false,
            change_fails: false,
        })
    }

    #[test]
    fn reads_the_current_scm_startup_type() {
        let startup = startup(Some(ServiceStartType::AutoStart));
        let port = port(startup.clone());
        assert!(port.is_enabled().unwrap());
        *startup.start_type.lock().unwrap() = Some(ServiceStartType::OnDemand);
        assert!(!port.is_enabled().unwrap());
    }

    #[test]
    fn disabling_autostart_preserves_manual_start() {
        let startup = startup(Some(ServiceStartType::AutoStart));
        let port = port(startup.clone());
        port.set_enabled(false).unwrap();
        assert_eq!(
            *startup.start_type.lock().unwrap(),
            Some(ServiceStartType::OnDemand)
        );
        assert!(!port.is_enabled().unwrap());
    }

    #[test]
    fn enabling_autostart_uses_automatic_start() {
        let startup = startup(Some(ServiceStartType::OnDemand));
        let port = port(startup.clone());
        port.set_enabled(true).unwrap();
        assert_eq!(
            *startup.start_type.lock().unwrap(),
            Some(ServiceStartType::AutoStart)
        );
        assert!(port.is_enabled().unwrap());
    }

    #[test]
    fn missing_service_is_unsupported_and_cannot_report_enabled() {
        let port = port(startup(None));
        assert!(!port.is_supported());
        assert!(port
            .is_enabled()
            .unwrap_err()
            .to_string()
            .contains("not installed"));
    }

    #[test]
    fn query_failure_is_not_disguised_as_unsupported() {
        let port = port(Arc::new(TestStartup {
            start_type: Mutex::new(Some(ServiceStartType::AutoStart)),
            query_fails: true,
            change_fails: false,
        }));
        assert!(port.is_supported());
        assert!(port
            .is_enabled()
            .unwrap_err()
            .to_string()
            .contains("query denied"));
    }

    #[test]
    fn change_failure_preserves_previous_configuration() {
        let startup = Arc::new(TestStartup {
            start_type: Mutex::new(Some(ServiceStartType::AutoStart)),
            query_fails: false,
            change_fails: true,
        });
        let port = port(startup.clone());
        assert!(port
            .set_enabled(false)
            .unwrap_err()
            .to_string()
            .contains("change denied"));
        assert_eq!(
            *startup.start_type.lock().unwrap(),
            Some(ServiceStartType::AutoStart)
        );
    }
}
