// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Windows' OLE drag, as the source: a data object that carries `CF_HDROP` (the file paths), a
//! drop source that ends the drag when the button is let go, and `DoDragDrop` over both. Explorer
//! (and every other OLE drop target) then copies the file where the owner drops it.

use std::os::windows::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{DV_E_FORMATETC, E_NOTIMPL, HGLOBAL, OLE_E_ADVISENOTSUPPORTED, POINT};
use windows::Win32::System::Com::{
    DATADIR_GET, FORMATETC, IAdviseSink, IDataObject, IDataObject_Impl, IEnumFORMATETC, IEnumSTATDATA, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{
    DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::DROPFILES;
use windows::core::{BOOL, HRESULT, Ref, Result as WinResult, implement};

/// `CF_HDROP`: the clipboard format of a list of files (a `DROPFILES` block).
const CF_HDROP: u16 = 15;
/// OLE's success codes for a drag that ended with a drop or was cancelled ([MS-OLEDS]).
const DRAGDROP_S_DROP: HRESULT = HRESULT(0x0004_0100u32 as i32);
const DRAGDROP_S_CANCEL: HRESULT = HRESULT(0x0004_0101u32 as i32);
const DRAGDROP_S_USEDEFAULTCURSORS: HRESULT = HRESULT(0x0004_0102u32 as i32);
/// `RPC_E_CHANGED_MODE`: OLE was already started on this thread in another apartment; a drag
/// still works there.
const RPC_E_CHANGED_MODE: HRESULT = HRESULT(0x8001_0106u32 as i32);

pub(super) fn start(paths: &[&Path]) -> Result<(), String> {
    let paths: Vec<PathBuf> = paths.iter().map(|path| path.to_path_buf()).collect();
    // OLE on this thread (the window's): a second call is an error to ignore.
    match unsafe { OleInitialize(None) } {
        Ok(()) => {}
        Err(why) if why.code() == RPC_E_CHANGED_MODE => {}
        Err(why) => return Err(why.message()),
    }
    let data: IDataObject = Drag { paths: paths.clone() }.into();
    let source: IDropSource = Drag { paths }.into();
    let mut effect = DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK;
    let done = unsafe { DoDragDrop(&data, &source, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK, &mut effect) };
    if done.is_err() {
        return Err(format!("the drag ended with {done:?}"));
    }
    Ok(())
}

/// The data object and the drop source in one: the files, the same however many times they are
/// asked for.
#[derive(Clone)]
#[implement(IDataObject, IDropSource)]
struct Drag {
    paths: Vec<PathBuf>,
}

impl Drag {
    /// A fresh `HGLOBAL` holding a `DROPFILES` block with the files, whose ownership passes to
    /// the caller (the drop target frees it).
    fn hdrop(&self) -> WinResult<HGLOBAL> {
        let mut wide: Vec<u16> = Vec::new();
        for path in &self.paths {
            wide.extend(path.as_os_str().encode_wide());
            wide.push(0);
        }
        wide.push(0);
        let header = core::mem::size_of::<DROPFILES>();
        let bytes = wide.len() * 2;
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, header + bytes)? };
        let memory = unsafe { GlobalLock(handle) } as *mut u8;
        if memory.is_null() {
            return Err(windows::core::Error::from_thread());
        }
        let block = DROPFILES { pFiles: header as u32, pt: POINT { x: 0, y: 0 }, fNC: BOOL(0), fWide: BOOL(1) };
        unsafe {
            core::ptr::write_unaligned(memory as *mut DROPFILES, block);
            core::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, memory.add(header), bytes);
        }
        let _ = unsafe { GlobalUnlock(handle) };
        Ok(handle)
    }

    fn has_hdrop(format: &FORMATETC) -> bool {
        format.cfFormat == CF_HDROP && (format.tymed == 0 || format.tymed & TYMED_HGLOBAL.0 as u32 != 0)
    }
}

impl IDataObject_Impl for Drag_Impl {
    fn GetData(&self, format: *const FORMATETC) -> WinResult<STGMEDIUM> {
        let format = unsafe { format.as_ref() }.ok_or_else(|| windows::core::Error::from(DV_E_FORMATETC))?;
        if !Drag::has_hdrop(format) {
            return Err(DV_E_FORMATETC.into());
        }
        let handle = self.hdrop()?;
        Ok(STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: handle }, pUnkForRelease: core::mem::ManuallyDrop::new(None) })
    }

    fn GetDataHere(&self, _format: *const FORMATETC, _medium: *mut STGMEDIUM) -> WinResult<()> {
        Err(E_NOTIMPL.into())
    }

    fn QueryGetData(&self, format: *const FORMATETC) -> HRESULT {
        match unsafe { format.as_ref() } {
            Some(format) if Drag::has_hdrop(format) => HRESULT(0),
            _ => DV_E_FORMATETC,
        }
    }

    fn GetCanonicalFormatEtc(&self, _format: *const FORMATETC, _out: *mut FORMATETC) -> HRESULT {
        E_NOTIMPL
    }

    fn SetData(&self, _format: *const FORMATETC, _medium: *const STGMEDIUM, _release: BOOL) -> WinResult<()> {
        Err(E_NOTIMPL.into())
    }

    fn EnumFormatEtc(&self, _direction: u32) -> WinResult<IEnumFORMATETC> {
        let _ = DATADIR_GET;
        Err(E_NOTIMPL.into())
    }

    fn DAdvise(&self, _format: *const FORMATETC, _flags: u32, _sink: Ref<IAdviseSink>) -> WinResult<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn DUnadvise(&self, _connection: u32) -> WinResult<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn EnumDAdvise(&self) -> WinResult<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}

impl IDropSource_Impl for Drag_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if keys.contains(MK_LBUTTON) {
            HRESULT(0)
        } else {
            DRAGDROP_S_DROP
        }
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}
