use std::{
    ffi::OsString,
    io::{self, Write},
    time::Duration,
};

use anyhow::{bail, ensure, Context, Result};
use mrd_ipc::{
    transport::{IpcClient, IpcEndpoint, IpcStream},
    IpcRequest, IpcResponse,
};
use mrd_service::windows_service::MRD_WINDOWS_SERVICE_NAME;
use windows::Win32::{
    Foundation::HANDLE,
    System::Console::{
        GetConsoleMode, GetStdHandle, ReadConsoleW, SetConsoleMode, CONSOLE_MODE,
        ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, STD_INPUT_HANDLE,
    },
};
use windows_service::{
    service::{ServiceAccess, ServiceState},
    service_manager::{ServiceManager, ServiceManagerAccess},
};
use zeroize::Zeroizing;

#[derive(Clone, Copy)]
pub(crate) enum PublicDeviceCommand {
    Enroll,
    Recover,
}

impl PublicDeviceCommand {
    pub(crate) fn from_args(arguments: &[OsString]) -> Result<Option<Self>> {
        let command = arguments.iter().find_map(|argument| {
            if argument == "--enroll-public-device" {
                Some(Self::Enroll)
            } else if argument == "--recover-public-device" {
                Some(Self::Recover)
            } else {
                None
            }
        });
        ensure!(
            if command.is_some() {
                arguments.len() == 1
            } else {
                !arguments.iter().any(|argument| {
                    let argument = argument.to_string_lossy();
                    argument.starts_with("--enroll-public-device=")
                        || argument.starts_with("--recover-public-device=")
                })
            },
            "The public device command accepts no other arguments"
        );
        Ok(command)
    }

    pub(crate) fn run(self) -> Result<()> {
        ensure!(
            mrd_service::agent_runtime::current_process_is_administrator()?,
            "Run this command from an elevated Windows terminal"
        );
        running_service_pid()?;
        let credential = read_credential(match self {
            Self::Enroll => "One-time enrollment code",
            Self::Recover => "Administrator-issued device recovery credential",
        })?;
        let request = match self {
            Self::Enroll => IpcRequest::EnrollPublicDevice {
                enrollment_token: credential.into(),
                device_name: std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Windows".into()),
            },
            Self::Recover => IpcRequest::RecoverPublicDevice {
                device_token: credential.into(),
            },
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let response = runtime.block_on(async move {
            tokio::time::timeout(Duration::from_secs(25), request_installed_service(request))
                .await
                .context("The Rdesk resident did not respond. Refresh registration status before retrying")?
        })?;
        match response {
            IpcResponse::PublicServerStatus { status } if status.device_registered => {
                println!(
                    "Device code: {}",
                    status.device_id.context("Missing device code")?
                );
                Ok(())
            }
            IpcResponse::Error { code, message } => bail!("{code}: {message}"),
            _ => bail!("The background service did not return a registered device"),
        }
    }
}

fn running_service_pid() -> Result<u32> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager
        .open_service(MRD_WINDOWS_SERVICE_NAME, ServiceAccess::QUERY_STATUS)
        .context("The Rdesk resident service must be installed")?;
    let status = service.query_status()?;
    ensure!(
        status.current_state == ServiceState::Running,
        "Start the installed Rdesk resident service first"
    );
    status
        .process_id
        .filter(|pid| *pid != 0)
        .context("Missing installed service process")
}

async fn request_installed_service(request: IpcRequest) -> Result<IpcResponse> {
    let endpoint = IpcEndpoint::management_for_service(IpcEndpoint::default_service());
    let mut stream = IpcClient::connect_product_with_endpoint(&endpoint)
        .await
        .context("Cannot authenticate the installed Rdesk resident service")?;
    let IpcStream::ProductClient(_, peer) = &stream else {
        bail!("The resident service pipe was not authenticated");
    };
    ensure!(
        peer.process_id() == running_service_pid()?,
        "The connected pipe does not belong to the running installed Rdesk service"
    );
    stream.send_request(&request).await?;
    stream.recv_response().await
}

struct RestoreConsoleMode(HANDLE, CONSOLE_MODE);

impl Drop for RestoreConsoleMode {
    fn drop(&mut self) {
        let _ = unsafe { SetConsoleMode(self.0, self.1) };
    }
}

fn read_credential(prompt: &str) -> Result<String> {
    const MAX_CREDENTIAL: usize = 4096;
    let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE)? };
    let mut mode = CONSOLE_MODE::default();
    unsafe { GetConsoleMode(handle, &mut mode) }.context("Use an interactive Windows console")?;
    unsafe { SetConsoleMode(handle, (mode | ENABLE_LINE_INPUT) & !ENABLE_ECHO_INPUT)? };
    let restore = RestoreConsoleMode(handle, mode);
    eprint!("{prompt}: ");
    io::stderr().flush()?;
    let mut input = Zeroizing::new(vec![0_u16; MAX_CREDENTIAL + 3]);
    let mut length = 0;
    let read = unsafe {
        ReadConsoleW(
            handle,
            input.as_mut_ptr().cast(),
            input.len() as u32,
            &mut length,
            None,
        )
    };
    drop(restore);
    eprintln!();
    read?;
    let length = length as usize;
    let line = input.get(..length).context("Invalid console input")?;
    let end = line.iter().position(|&unit| unit == 13 || unit == 10);
    let end = end.filter(|&end| (1..=MAX_CREDENTIAL).contains(&end));
    let end = end.context("Enter a non-empty credential of at most 4096 characters")?;
    String::from_utf16(&line[..end]).context("Invalid credential encoding")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_is_prompt_only() {
        assert!(matches!(
            PublicDeviceCommand::from_args(&[OsString::from("--recover-public-device")]).unwrap(),
            Some(PublicDeviceCommand::Recover)
        ));
        assert!(PublicDeviceCommand::from_args(&[
            "--enroll-public-device".into(),
            "fixture-credential".into()
        ])
        .is_err());
    }
}
