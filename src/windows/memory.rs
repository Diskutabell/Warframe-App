//! Windows: the game's memory through the Win32 debugging API, read-only.
//!
//! The same three steps as on Linux: find the process, list the memory regions the inventory text
//! can be in, and copy bytes out of them. `OpenProcess` asks only for the rights to look and to
//! read (`PROCESS_QUERY_INFORMATION | PROCESS_VM_READ`), so nothing here could write to the game
//! even by mistake.

use std::ffi::c_void;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_PRIVATE, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_READWRITE, VirtualQueryEx,
};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

use super::ScanError;

/// Nothing to rule out up front: only opening the process can tell.
pub fn availability() -> Result<(), String> {
    Ok(())
}

/// The process id of `Warframe.x64.exe` (not the launcher).
pub fn find_game_pid() -> Option<u32> {
    // SAFETY: plain Win32 calls; the entry is a zeroed struct with its size filled in, as the API
    // asks, and the snapshot handle is closed on every path.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut found = None;
        let mut more = Process32FirstW(snap, &mut entry) != 0;
        while more {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if name.eq_ignore_ascii_case("Warframe.x64.exe") {
                found = Some(entry.th32ProcessID);
                break;
            }
            more = Process32NextW(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
        found
    }
}

/// The game's memory, opened for reading. The handle is closed when this is dropped.
pub struct Memory {
    process: HANDLE,
}

// SAFETY: a process handle may be used from any thread.
unsafe impl Send for Memory {}

impl Memory {
    pub fn open(pid: u32) -> Result<Memory, ScanError> {
        // SAFETY: OpenProcess has no preconditions; a null handle means it failed.
        let process = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
        if process.is_null() {
            // SAFETY: reads the calling thread's last error code.
            let code = unsafe { GetLastError() };
            return Err(match code {
                ERROR_ACCESS_DENIED => ScanError::Denied(
                    "Windows refused access to the game's memory. It is tried again when the game restarts.".into(),
                ),
                // No process with that id any more.
                ERROR_INVALID_PARAMETER => ScanError::NotRunning,
                _ => ScanError::Io(format!("Windows error {code}")),
            });
        }
        Ok(Memory { process })
    }

    /// Committed, private, read-write regions: the heap, where downloaded JSON lives. Mapped
    /// files and the program's own code are left out.
    pub fn regions(&self) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        let mut addr: usize = 0;
        loop {
            // SAFETY: VirtualQueryEx fills `info` and returns how many bytes it wrote; 0 means
            // the address is past the end of the address space.
            let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
            let got = unsafe {
                VirtualQueryEx(self.process, addr as *const c_void, &mut info, size_of::<MEMORY_BASIC_INFORMATION>())
            };
            if got == 0 || info.RegionSize == 0 {
                break;
            }
            let (start, size) = (info.BaseAddress as u64, info.RegionSize as u64);
            let readable = info.State == MEM_COMMIT
                && info.Type == MEM_PRIVATE
                && info.Protect & PAGE_READWRITE != 0
                && info.Protect & PAGE_GUARD == 0;
            if readable {
                out.push((start, start + size));
            }
            match (info.BaseAddress as usize).checked_add(info.RegionSize) {
                Some(next) if next > addr => addr = next,
                _ => break,
            }
        }
        out
    }

    /// Up to `len` bytes at `addr`; fewer (or none) where the memory is not readable.
    pub fn read(&self, addr: u64, len: usize) -> Vec<u8> {
        let mut buf = vec![0u8; len];
        let mut got: usize = 0;
        // SAFETY: the buffer is `len` bytes long, and ReadProcessMemory writes at most that many.
        // A partial read (the region ended) still reports how many bytes it copied.
        unsafe {
            ReadProcessMemory(self.process, addr as *const c_void, buf.as_mut_ptr().cast(), len, &mut got);
        }
        buf.truncate(got);
        buf
    }
}

impl Drop for Memory {
    fn drop(&mut self) {
        // SAFETY: the handle came from OpenProcess and is closed exactly once.
        unsafe {
            CloseHandle(self.process);
        }
    }
}
