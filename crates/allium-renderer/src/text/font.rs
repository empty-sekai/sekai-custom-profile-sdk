use skia_safe::{FontMgr, FontStyle, Typeface};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy)]
pub(super) struct TmpFaceInfoConstants {
    pub point_size: f32,
    pub m_scale: f32,
    pub superscript_offset: f32,
    pub superscript_size: f32,
    pub subscript_offset: f32,
    pub subscript_size: f32,
    /// m_CapLine: italic shear midPoint 计算用
    pub cap_line: f32,
}

fn typeface_cache() -> &'static Mutex<HashMap<String, Option<Typeface>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Typeface>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn face_info_cache() -> &'static Mutex<HashMap<String, Option<TmpFaceInfoConstants>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<TmpFaceInfoConstants>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn resolve_tmp_face_info_constants(family: Option<&str>) -> TmpFaceInfoConstants {
    const DEFAULTS: TmpFaceInfoConstants = TmpFaceInfoConstants {
        point_size: 75.0,
        m_scale: 2.0,
        superscript_offset: 66.0,
        superscript_size: 0.5,
        subscript_offset: -9.0,
        subscript_size: 0.5,
        cap_line: 60.0,
    };

    let Some(family) = family else {
        return DEFAULTS;
    };

    if let Some(cached) = face_info_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(family).copied())
    {
        return cached.unwrap_or(DEFAULTS);
    }

    let candidates = [
        PathBuf::from("tmp/jp_font_extract_bootstrap_live/exported").join(format!("{family}.json")),
        PathBuf::from("tmp/font_extract/exported").join(format!("{family}.json")),
    ];

    let resolved = candidates.iter().find_map(|path| {
        let text = std::fs::read_to_string(path).ok()?;
        let json: serde_json::Value = serde_json::from_str(&text).ok()?;
        let face = json.get("m_FaceInfo")?;
        Some(TmpFaceInfoConstants {
            point_size: face.get("m_PointSize")?.as_f64()? as f32,
            m_scale: face.get("m_Scale")?.as_f64()? as f32,
            superscript_offset: face.get("m_SuperscriptOffset")?.as_f64()? as f32,
            superscript_size: face.get("m_SuperscriptSize")?.as_f64()? as f32,
            subscript_offset: face.get("m_SubscriptOffset")?.as_f64()? as f32,
            subscript_size: face.get("m_SubscriptSize")?.as_f64()? as f32,
            cap_line: face.get("m_CapLine").and_then(|v| v.as_f64()).unwrap_or(60.0) as f32,
        })
    });

    if let Ok(mut cache) = face_info_cache().lock() {
        cache.insert(family.to_string(), resolved);
    }

    resolved.unwrap_or(DEFAULTS)
}

pub(super) fn resolve_same_source_typeface(
    font_mgr: &FontMgr,
    family: Option<&str>,
) -> Option<Typeface> {
    let family = family?;

    if let Some(cached) = typeface_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(family).cloned())
    {
        return cached;
    }

    let resolved = crate::sdf::outline::load_font_bytes_for_family(family)
        .and_then(|bytes| font_mgr.new_from_data(bytes.as_slice(), None));

    if let Ok(mut cache) = typeface_cache().lock() {
        cache.insert(family.to_string(), resolved.clone());
    }

    resolved
}

pub(super) fn resolve_typeface(font_mgr: &FontMgr, family: Option<&str>) -> Option<Typeface> {
    resolve_same_source_typeface(font_mgr, family).or_else(|| {
        family.and_then(|name| {
            let tf = font_mgr.match_family_style(name, FontStyle::default());
            if let Some(ref t) = tf {
                tracing::trace!(
                    requested = name,
                    resolved = %t.family_name(),
                    "字体匹配"
                );
            }
            tf
        })
    })
}
