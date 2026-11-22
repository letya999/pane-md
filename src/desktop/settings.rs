use super::{rich::wide, theme::EditorAppearance};
use windows::{
    Win32::{Foundation::ERROR_SUCCESS, System::Registry::*},
    core::{PCWSTR, w},
};

const SETTINGS: PCWSTR = w!("Software\\pane-md");
const LEGACY_SETTINGS: PCWSTR = w!("Software\\notepad-md");
const APPEARANCE: PCWSTR = w!("EditorAppearance");

pub(super) fn read_preference(name: &str) -> Option<u32> {
    let name = wide(name);
    read_value(SETTINGS, PCWSTR(name.as_ptr()))
        .or_else(|| read_value(LEGACY_SETTINGS, PCWSTR(name.as_ptr())))
}

fn read_value(path: PCWSTR, name: PCWSTR) -> Option<u32> {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            path,
            name,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut _ as _),
            Some(&mut size),
        )
    };
    (result == ERROR_SUCCESS).then_some(value)
}

pub(super) fn write_preference(name: &str, value: u32) {
    let name = wide(name);
    let _ = write_value(
        SETTINGS,
        PCWSTR(name.as_ptr()),
        REG_DWORD,
        (&value as *const u32).cast(),
        size_of_val(&value) as u32,
    );
}

fn write_value(
    path: PCWSTR,
    name: PCWSTR,
    kind: REG_VALUE_TYPE,
    data: *const std::ffi::c_void,
    bytes: u32,
) -> windows::core::Result<()> {
    let mut key = HKEY::default();
    unsafe {
        RegCreateKeyW(HKEY_CURRENT_USER, path, &mut key).ok()?;
        let result = RegSetKeyValueW(key, PCWSTR::null(), name, kind.0, Some(data), bytes);
        let _ = RegCloseKey(key);
        result.ok()
    }
}

pub(super) fn load_appearance() -> EditorAppearance {
    read_appearance(SETTINGS)
        .or_else(|| read_appearance(LEGACY_SETTINGS))
        .unwrap_or_default()
}

fn read_appearance(path: PCWSTR) -> Option<EditorAppearance> {
    let mut value = [0u16; 2048];
    let mut bytes = size_of_val(&value) as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            path,
            APPEARANCE,
            RRF_RT_REG_SZ,
            None,
            Some(value.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if result != ERROR_SUCCESS {
        return None;
    }
    let text = String::from_utf16(&value[..value.iter().position(|&unit| unit == 0)?]).ok()?;
    let mut appearance: EditorAppearance = serde_json::from_str(&text).ok()?;
    appearance.validate();
    Some(appearance)
}

pub(super) fn save_appearance(appearance: &EditorAppearance) -> crate::Result<()> {
    write_appearance(SETTINGS, appearance)
}

fn write_appearance(path: PCWSTR, appearance: &EditorAppearance) -> crate::Result<()> {
    let encoded = wide(&serde_json::to_string(appearance)?);
    write_value(
        path,
        APPEARANCE,
        REG_SZ,
        encoded.as_ptr().cast(),
        size_of_val(encoded.as_slice()) as u32,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::theme::EditorFont;

    #[test]
    fn appearance_round_trips_through_an_isolated_registry_key() -> crate::Result<()> {
        let path = wide(&format!("Software\\pane-md-test-{}", uuid::Uuid::new_v4()));
        let path = PCWSTR(path.as_ptr());
        struct RemoveKey(PCWSTR);
        impl Drop for RemoveKey {
            fn drop(&mut self) {
                unsafe {
                    let _ = RegDeleteKeyW(HKEY_CURRENT_USER, self.0);
                }
            }
        }
        let _cleanup = RemoveKey(path);
        assert!(read_appearance(path).is_none());
        let mut appearance = EditorAppearance {
            rich: EditorFont::new("メイリオ".into(), 30).unwrap(),
            ..EditorAppearance::default()
        };
        appearance.light.paper = Some(0xf4eedf);
        appearance.dark.link = Some(0xffca64);
        write_appearance(path, &appearance)?;
        assert_eq!(read_appearance(path), Some(appearance.clone()));

        appearance.source.face = r"Not a font;}\b".into();
        appearance.light.ink = Some(u32::MAX);
        write_appearance(path, &appearance)?;
        appearance.validate();
        assert_eq!(appearance.source, EditorAppearance::default().source);
        assert_eq!(appearance.light.ink, None);
        assert_eq!(read_appearance(path), Some(appearance));
        Ok(())
    }
}
