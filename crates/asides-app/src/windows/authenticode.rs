//! A file's Authenticode signature, as Windows checks it: valid, chained to a root this PC trusts,
//! and by whom. An update's installer must be signed by whoever signed the copy running (`update.rs`).
//!
//! `WinVerifyTrust` with the generic Authenticode policy, without UI and without revocation checks
//! (no network needed), then the signing certificate's whole subject (`CN=…, O=…, C=…`, as
//! `Get-AuthenticodeSignature` writes it): the subject, not a thumbprint, since a renewed
//! certificate keeps it. As helio checks its installers.

use std::path::Path;

use windows::Win32::Foundation::{HANDLE, HWND, INVALID_HANDLE_VALUE, TRUST_E_BAD_DIGEST, TRUST_E_NOSIGNATURE};
use windows::Win32::Security::Cryptography::{
    CERT_NAME_RDN_TYPE, CERT_NAME_STR_REVERSE_FLAG, CERT_X500_NAME_STR, CertGetNameStringW,
};
use windows::Win32::Security::WinTrust::{
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO, WTD_CHOICE_FILE,
    WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE, WTHelperGetProvCertFromChain,
    WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData, WinVerifyTrust,
};
use windows::core::{HRESULT, HSTRING, PCWSTR};

/// The publisher whose valid, trusted signature `file` carries, or what's wrong with it, for a person.
pub fn signer(file: &Path) -> Result<String, String> {
    let path = HSTRING::from(file.as_os_str());
    let mut info = WINTRUST_FILE_INFO {
        cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(path.as_ptr()),
        ..Default::default()
    };
    let mut data = WINTRUST_DATA {
        cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 { pFile: &mut info },
        dwStateAction: WTD_STATEACTION_VERIFY,
        ..Default::default()
    };
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // No window: no UI may be shown (`WTD_UI_NONE`).
    let no_window = HWND(INVALID_HANDLE_VALUE.0);
    // SAFETY: `data` points at `info`, which points at `path`; all three outlive both calls. The
    // state the first call keeps is released by the second (WTD_STATEACTION_CLOSE), whatever the
    // first found.
    let verdict = unsafe {
        let verdict = WinVerifyTrust(no_window, &mut action, (&raw mut data).cast());
        let publisher = (verdict == 0).then(|| publisher_of(data.hWVTStateData));
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        WinVerifyTrust(no_window, &mut action, (&raw mut data).cast());
        match publisher {
            Some(p) => Ok(p),
            None => Err(HRESULT(verdict)),
        }
    };
    match verdict {
        Ok(Some(publisher)) => Ok(publisher),
        Ok(None) => Err("its signature names no publisher".into()),
        Err(e) if e == TRUST_E_NOSIGNATURE => Err("it isn't signed".into()),
        Err(e) if e == TRUST_E_BAD_DIGEST => Err("it was changed after it was signed".into()),
        Err(e) => Err(format!("Windows doesn't trust its signature ({})", e.message())),
    }
}

/// The signing certificate's subject, the most specific part first, from what a successful
/// `WinVerifyTrust` keeps.
///
/// # Safety
/// `state` is the `hWVTStateData` of a verification that succeeded and isn't closed yet.
unsafe fn publisher_of(state: HANDLE) -> Option<String> {
    // SAFETY: per the contract; each pointer is checked before it's read, and all of them belong to
    // `state`, alive until it's closed.
    unsafe {
        let provider = WTHelperProvDataFromStateData(state);
        if provider.is_null() {
            return None;
        }
        let signer = WTHelperGetProvSignerFromChain(provider, 0, false, 0);
        if signer.is_null() {
            return None;
        }
        let cert = WTHelperGetProvCertFromChain(signer, 0);
        if cert.is_null() || (*cert).pCert.is_null() {
            return None;
        }
        let format = CERT_X500_NAME_STR.0 | CERT_NAME_STR_REVERSE_FLAG;
        let mut name = [0u16; 1024];
        let len =
            CertGetNameStringW((*cert).pCert, CERT_NAME_RDN_TYPE, 0, Some((&raw const format).cast()), Some(&mut name))
                as usize;
        // The length counts the NUL; 1 is an empty name.
        (len > 1).then(|| String::from_utf16_lossy(&name[..len - 1]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A program this PC trusts, signed in the file by a publisher that isn't asides': Microsoft
    /// Edge, which Windows 10 and 11 come with (most of System32 is signed through catalogs, which
    /// a file's own check doesn't see).
    fn someone_elses_program() -> Option<std::path::PathBuf> {
        let x86 = std::env::var_os("ProgramFiles(x86)")?;
        let edge = Path::new(&x86).join(r"Microsoft\Edge\Application\msedge.exe");
        edge.is_file().then_some(edge)
    }

    #[test]
    fn a_trusted_program_names_its_publisher() {
        let Some(edge) = someone_elses_program() else { return };
        let publisher = signer(&edge).unwrap();
        assert!(publisher.starts_with("CN=Microsoft Corporation"), "{publisher}");
    }

    #[test]
    fn an_unsigned_file_has_no_publisher() {
        let file = tempfile::Builder::new().suffix(".exe").tempfile().unwrap();
        std::fs::write(file.path(), b"MZ not really a program").unwrap();
        assert!(signer(file.path()).is_err());
    }

    #[test]
    fn a_signed_file_changed_after_signing_is_refused() {
        let Some(edge) = someone_elses_program() else { return };
        let dir = tempfile::tempdir().unwrap();
        let copy = dir.path().join("msedge.exe");
        let mut bytes = std::fs::read(&edge).unwrap();
        // A byte in the middle of the code, which the signature covers.
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0xff;
        std::fs::write(&copy, bytes).unwrap();
        let err = signer(&copy).unwrap_err();
        assert!(err.contains("changed after it was signed") || err.contains("doesn't trust"), "{err}");
    }
}
