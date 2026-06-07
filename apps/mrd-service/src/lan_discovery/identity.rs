pub(crate) const SERVICE_BUILD_ID_ENV: &str = "MRD_SERVICE_BUILD_ID";

pub(crate) fn service_build_id_from_lookup(lookup: impl Fn(&str) -> Option<String>) -> String {
    if let Some(value) = lookup(SERVICE_BUILD_ID_ENV) {
        let value = value.trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }

    option_env!("VERGEN_GIT_SHA")
        .or(option_env!("GIT_COMMIT"))
        .unwrap_or(env!("CARGO_PKG_VERSION"))
        .to_string()
}
