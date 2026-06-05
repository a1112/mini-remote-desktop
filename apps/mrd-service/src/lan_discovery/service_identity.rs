pub(super) const SERVICE_BUILD_ID_ENV: &str = "MRD_SERVICE_BUILD_ID";

pub(super) fn service_build_id() -> String {
    service_build_id_from_lookup(|key| std::env::var(key).ok())
}

pub(super) fn service_build_id_from_lookup(lookup: impl Fn(&str) -> Option<String>) -> String {
    if let Some(value) = lookup(SERVICE_BUILD_ID_ENV) {
        let value = value.trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }

    compile_time_service_build_id()
}

fn compile_time_service_build_id() -> String {
    option_env!("VERGEN_GIT_SHA")
        .or(option_env!("GIT_COMMIT"))
        .unwrap_or(env!("CARGO_PKG_VERSION"))
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_build_id_prefers_runtime_override() {
        let build_id = service_build_id_from_lookup(|key| {
            if key == SERVICE_BUILD_ID_ENV {
                Some("peer-runtime-build".to_string())
            } else {
                None
            }
        });

        assert_eq!(build_id, "peer-runtime-build");
    }

    #[test]
    fn service_build_id_ignores_blank_runtime_override() {
        let build_id = service_build_id_from_lookup(|key| {
            if key == SERVICE_BUILD_ID_ENV {
                Some("   ".to_string())
            } else {
                None
            }
        });

        assert_eq!(build_id, compile_time_service_build_id());
    }
}
