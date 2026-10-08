// Diagnostics only (tests): captures a program's sound the way Discord's screen share and OBS's
// Application Audio Capture do (Windows' process loopback), to see whether OmniPulse can be heard.
//   cargo test -p omnipulse-core capture_omnipulse -- --ignored --nocapture
// Nothing is kept: it only measures how loud the captured sound is.

#![cfg(all(test, windows))]

use std::sync::mpsc;
use windows::core::{Interface, Ref, Result, HRESULT};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Diagnostics::ToolHelp::*;

#[windows_core::implement(IActivateAudioInterfaceCompletionHandler, IAgileObject)]
struct Done(mpsc::Sender<()>);

impl IAgileObject_Impl for Done_Impl {}

impl IActivateAudioInterfaceCompletionHandler_Impl for Done_Impl {
    fn ActivateCompleted(&self, _op: Ref<IActivateAudioInterfaceAsyncOperation>) -> Result<()> {
        let _ = self.0.send(());
        Ok(())
    }
}

#[repr(C)]
struct BlobVariant {
    vt: u16,
    r1: u16,
    r2: u16,
    r3: u16,
    size: u32,
    data: *mut u8,
}

/// The loudest sample heard from a process (and, with `tree`, the programs it started) over `ms`
fn capture_peak(pid: u32, tree: bool, ms: u64) -> std::result::Result<(f32, usize), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: pid,
                    ProcessLoopbackMode: if tree { PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE } else { PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE },
                },
            },
        };
        let pv = BlobVariant { vt: 65 /* VT_BLOB */, r1: 0, r2: 0, r3: 0, size: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32, data: &mut params as *mut _ as *mut u8 };
        let (tx, rx) = mpsc::channel();
        let handler: IActivateAudioInterfaceCompletionHandler = Done(tx).into();
        let op = ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&pv as *const BlobVariant as *const windows::Win32::System::Com::StructuredStorage::PROPVARIANT),
            &handler,
        )
        .map_err(|e| e.to_string())?;
        rx.recv_timeout(std::time::Duration::from_secs(5)).map_err(|_| "activation timed out")?;
        let mut hr = HRESULT(0);
        let mut unk = None;
        op.GetActivateResult(&mut hr, &mut unk).map_err(|e| e.to_string())?;
        hr.ok().map_err(|e| e.to_string())?;
        let client: IAudioClient = unk.ok_or("no client")?.cast().map_err(|e| e.to_string())?;
        // process loopback has no mix format of its own: ask for 48 kHz stereo float
        let fmt = WAVEFORMATEX { wFormatTag: 3, nChannels: 2, nSamplesPerSec: 48000, nAvgBytesPerSec: 48000 * 8, nBlockAlign: 8, wBitsPerSample: 32, cbSize: 0 };
        client
            .Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, 2_000_000, 0, &fmt, None)
            .map_err(|e| e.to_string())?;
        let cap: IAudioCaptureClient = client.GetService().map_err(|e| e.to_string())?;
        client.Start().map_err(|e| e.to_string())?;
        let (mut peak, mut frames) = (0f32, 0usize);
        let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
        while std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(20));
            while cap.GetNextPacketSize().unwrap_or(0) > 0 {
                let mut data = std::ptr::null_mut();
                let mut n = 0u32;
                let mut flags = 0u32;
                if cap.GetBuffer(&mut data, &mut n, &mut flags, None, None).is_err() {
                    break;
                }
                if flags & 2 == 0 && !data.is_null() {
                    let s = std::slice::from_raw_parts(data as *const f32, n as usize * 2);
                    peak = s.iter().fold(peak, |m, v| m.max(v.abs()));
                }
                frames += n as usize;
                let _ = cap.ReleaseBuffer(n);
            }
        }
        let _ = client.Stop();
        Ok((peak, frames))
    }
}

fn pids_named(name: &str) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).unwrap();
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snap, &mut e).is_ok() {
            loop {
                let n = String::from_utf16_lossy(&e.szExeFile[..e.szExeFile.iter().position(|&c| c == 0).unwrap_or(0)]);
                if n.eq_ignore_ascii_case(name) {
                    out.push((e.th32ProcessID, e.th32ParentProcessID));
                }
                if Process32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
    }
    out
}

#[test]
#[ignore]
fn capture_omnipulse() {
    let apps = pids_named("OmniPulse.exe");
    println!("OmniPulse.exe: {apps:?}");
    for (pid, _) in &apps {
        println!("OmniPulse.exe {pid} with the programs it started: {:?}", capture_peak(*pid, true, 2000));
        println!("OmniPulse.exe {pid} on its own: {:?}", capture_peak(*pid, false, 1500));
    }
    let views = pids_named("msedgewebview2.exe");
    let ours: Vec<_> = views.iter().filter(|(_, parent)| apps.iter().any(|(a, _)| a == parent)).collect();
    println!("its WebView2 (started by OmniPulse): {ours:?}");
    for (pid, _) in ours {
        println!("WebView2 {pid} with its children: {:?}", capture_peak(*pid, true, 2000));
    }
}
