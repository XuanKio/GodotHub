use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    process::Command,
};

pub fn executable_arch(path: &Path) -> Result<&'static str, String> {
    fn machine(path: &Path) -> std::io::Result<u16> {
        let mut file = File::open(path)?;
        let mut header = [0; 64];
        file.read_exact(&mut header)?;
        if &header[..2] != b"MZ" {
            return Err(std::io::Error::other("Missing DOS executable header"));
        }
        let offset = u32::from_le_bytes(header[60..64].try_into().unwrap());
        file.seek(SeekFrom::Start(u64::from(offset)))?;
        let mut header = [0; 6];
        file.read_exact(&mut header)?;
        if &header[..4] != b"PE\0\0" {
            return Err(std::io::Error::other("Missing Windows executable header"));
        }
        Ok(u16::from_le_bytes([header[4], header[5]]))
    }

    match machine(path).map_err(|e| format!("Cannot read executable {}: {e}", path.display()))? {
        0x014c => Ok("x86"),
        0x8664 => Ok("x64"),
        0xaa64 => Ok("arm64"),
        _ => Err("This executable uses an unsupported processor architecture.".into()),
    }
}

#[cfg(windows)]
pub fn native_arch() -> &'static str {
    use windows::Win32::System::SystemInformation::{GetNativeSystemInfo, SYSTEM_INFO};
    let mut info = SYSTEM_INFO::default();
    unsafe {
        GetNativeSystemInfo(&mut info);
        match info.Anonymous.Anonymous.wProcessorArchitecture.0 {
            12 => "arm64",
            0 => "x86",
            _ => "x64",
        }
    }
}

pub fn compatible(arch: &str) -> bool {
    arch == native_arch() || arch == "x86" || (native_arch() == "arm64" && arch == "x64")
}

pub fn reveal(path: &Path) -> Result<(), String> {
    if !path.is_dir() {
        return Err(format!("Folder not found: {}", path.display()));
    }
    Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Cannot open File Explorer: {e}"))
}

#[cfg(windows)]
pub fn create_shortcut(executable: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::{Interface, PCWSTR},
        Win32::{
            System::Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile,
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
            },
            UI::Shell::{IShellLinkW, ShellLink},
        },
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    fn create(executable: &Path, destination: &Path) -> windows::core::Result<()> {
        let target = wide(executable);
        let folder = wide(executable.parent().expect("executable path has a parent"));
        let link_path = wide(destination);
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe { CoUninitialize() };
            }
        }
        let _apartment = Apartment;
        unsafe {
            let shortcut: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            shortcut.SetPath(PCWSTR(target.as_ptr()))?;
            shortcut.SetWorkingDirectory(PCWSTR(folder.as_ptr()))?;
            shortcut.SetIconLocation(PCWSTR(target.as_ptr()), 0)?;
            shortcut
                .cast::<IPersistFile>()?
                .Save(PCWSTR(link_path.as_ptr()), true)
        }
    }

    create(executable, destination)
        .map_err(|e| format!("Cannot create shortcut at {}: {e}", destination.display()))
}
