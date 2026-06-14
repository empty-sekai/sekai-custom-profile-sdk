//! 全局样式常量。
//!
//! 所有图元组件的视觉参数集中管理。
//! 值来源标注在行末注释中，便于后续用游戏原生值替换。

use serde::{Deserialize, Serialize};

/// RGBA 颜色对象。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Color {
    /// 红色通道，范围 `0.0..=1.0`。
    pub r: f32,
    /// 绿色通道，范围 `0.0..=1.0`。
    pub g: f32,
    /// 蓝色通道，范围 `0.0..=1.0`。
    pub b: f32,
    /// 透明度通道，范围 `0.0..=1.0`。
    pub a: f32,
}

impl Color {
    /// 创建颜色对象。
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// 由 8-bit RGBA 创建颜色对象。
    pub const fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: a as f32 / 255.0,
        }
    }

    /// 转换为 Skia `Color4f`。
    #[cfg(feature = "skia-core")]
    pub fn to_skia(self) -> skia_safe::Color4f {
        skia_safe::Color4f::new(self.r, self.g, self.b, self.a)
    }
}

/// 主题色板。
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// 25 时主色。
    pub niigo_purple: Color,
    /// 初音青色。
    pub miku_cyan: Color,
    /// 加成橙色。
    pub bonus_orange: Color,
    /// 深色背景。
    pub bg_dark: Color,
    /// 白色正文。
    pub text_white: Color,
    /// 灰色副文本。
    pub text_gray: Color,
    /// 玻璃底色。
    pub glass_bg: Color,
    /// 玻璃边框。
    pub glass_edge: Color,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            niigo_purple: colors::NIIGO_PURPLE,
            miku_cyan: colors::MIKU_CYAN,
            bonus_orange: colors::BONUS_ORANGE,
            bg_dark: colors::BG_DARK,
            text_white: colors::TEXT_WHITE,
            text_gray: colors::TEXT_GRAY,
            glass_bg: colors::GLASS_BG,
            glass_edge: colors::GLASS_EDGE,
        }
    }
}

/// Widget 默认主题。
#[derive(Debug, Clone, Copy, Default)]
pub struct Theme {
    /// 颜色色板。
    pub colors: Palette,
}

/// 主色板（来自旧 bot deck_result.html CSS 变量）。
pub mod colors {
    use super::Color;

    pub const NIIGO_PURPLE: Color = Color::new(0.533, 0.478, 0.941, 1.0); // 旧 bot CSS: #887AF0
    pub const MIKU_CYAN: Color = Color::new(0.659, 0.847, 0.910, 1.0); // 旧 bot CSS: #A8D8E8
    pub const BONUS_ORANGE: Color = Color::new(1.0, 0.624, 0.263, 1.0); // 旧 bot CSS: #FF9F43
    pub const BG_DARK: Color = Color::new(0.067, 0.067, 0.102, 1.0); // 旧 bot CSS: #111119
    pub const TEXT_WHITE: Color = Color::new(1.0, 1.0, 1.0, 1.0); // 旧 bot CSS: 白色正文
    pub const TEXT_GRAY: Color = Color::new(0.6, 0.6, 0.65, 1.0); // 旧 bot CSS: 次级灰字
    pub const GLASS_BG: Color = Color::new(0.94, 0.96, 1.0, 0.08); // 旧 bot CSS: 玻璃底色
    pub const GLASS_EDGE: Color = Color::new(1.0, 1.0, 1.0, 0.3); // 旧 bot CSS: 玻璃描边
}

/// 字体名称常量（优先复用已安装的 3 个游戏字体）。
pub mod fonts {
    /// 主字体（游戏内标准 UI 字体）。
    pub const PRIMARY: &str = "FZLanTingHei-DB-GBK";
    /// 粗体强调字体。
    pub const EMPHASIS: &str = "FZZhengHei-EB-GBK";
    /// 回退字体。
    pub const FALLBACK: &str = "sans-serif";
}

/// 卡面缩略图参数（游戏原生 UIPartsCardThumbnail 实测值）。
///
/// 来源：`docs/game_ui_assets_spec.md` §2.2，RectTransform 实测。
/// 游戏原生根尺寸 = 156×156，下列比例基于此。
/// 渲染时按输出目标尺寸等比缩放即可。
pub mod card_thumbnail {
    /// 游戏原生根尺寸（DeckCardThumbnail 实测）。
    pub const NATIVE_SIZE: i32 = 156;
    /// 属性图标宽度比例：34.23 / 156 = 0.2194。
    pub const ATTR_RATIO_W: f32 = 0.219; // 游戏 34.23 / 156
    /// 属性图标高度比例：36.36 / 156 = 0.2331。
    pub const ATTR_RATIO_H: f32 = 0.233; // 游戏 36.36 / 156
    /// 属性图标左上锚点偏移：x=2px → 2/156。
    pub const ATTR_OFFSET_X: f32 = 0.013; // 游戏 x=2
    /// 突破等级尺寸比例：54 / 156 = 0.3462。
    pub const RANK_RATIO: f32 = 0.346; // 游戏 54 / 156
    /// 单颗星尺寸比例：22 / 156 = 0.1410。
    pub const STAR_RATIO: f32 = 0.141; // 游戏 22 / 156
    /// 星级容器左偏移：4.34 / 156 = 0.0278。
    pub const STAR_H_OFFSET: f32 = 0.028; // 游戏 x=4.34
    /// 星级容器底偏移：33.73 / 156 = 0.2162。
    pub const STAR_V_OFFSET: f32 = 0.216; // 游戏 y=33.73
    /// 圆角由 FrameMask 裁切，此值为 Skia 回退用。
    pub const CORNER_RADIUS: f32 = 10.0; // Skia 回退
}

/// 玻璃面板参数。
pub mod glass_panel {
    pub const BLUR_SIGMA: f32 = 10.0; // CSS blur(20px) -> sigma 约 10
    pub const SHADOW_ALPHA: f32 = 0.5; // 旧 bot 玻璃阴影透明度近似
}

#[cfg(test)]
mod tests {
    use super::Color;

    #[test]
    fn color_round_trip_via_serde() {
        let color = Color::new(1.0, 0.5, 0.0, 1.0);
        let json = serde_json::to_string(&color).expect("序列化颜色失败");
        let decoded: Color = serde_json::from_str(&json).expect("反序列化颜色失败");

        assert_eq!(json, r#"{"r":1.0,"g":0.5,"b":0.0,"a":1.0}"#);
        assert_eq!(decoded, color);
    }
}
