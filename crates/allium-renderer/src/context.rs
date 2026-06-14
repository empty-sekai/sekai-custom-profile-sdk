//! Widget 渲染上下文。
//!
//! 运行时注入素材仓库与主题，避免 Widget trait 携带关联数据类型。

use crate::assets::AssetStore;
use crate::masterdata::MasterData;
use crate::profile::ProfileData;
use crate::widgets::theme::Theme;

/// Widget 渲染上下文。
///
/// 由上层渲染器在调用 `Widget::measure()` / `Widget::draw()` 时构造，
/// 用于注入运行时依赖。
pub struct RenderContext<'a> {
    /// 渲染素材仓库。
    pub assets: &'a AssetStore,
    /// 当前主题快照。
    pub theme: &'a Theme,
    /// 当前渲染使用的 MasterData 快照。
    pub masterdata: Option<&'a MasterData>,
    /// 当前渲染使用的玩家资料快照。
    pub profile: Option<&'a ProfileData>,
}

impl<'a> RenderContext<'a> {
    /// 创建渲染上下文。
    pub fn new(assets: &'a AssetStore, theme: &'a Theme) -> Self {
        Self {
            assets,
            theme,
            masterdata: None,
            profile: None,
        }
    }

    /// 注入 MasterData 快照。
    pub fn with_masterdata(mut self, masterdata: &'a MasterData) -> Self {
        self.masterdata = Some(masterdata);
        self
    }

    /// 注入玩家资料快照。
    pub fn with_profile(mut self, profile: &'a ProfileData) -> Self {
        self.profile = Some(profile);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::RenderContext;
    use crate::assets::AssetStore;
    use crate::widgets::theme::Theme;

    #[test]
    fn render_context_can_be_constructed() {
        let assets = AssetStore::new(8);
        let theme = Theme::default();
        let ctx = RenderContext::new(&assets, &theme);

        assert!(!ctx.assets.contains("missing"));
        assert_eq!(ctx.theme.colors.text_white.a, 1.0);
        assert!(ctx.masterdata.is_none());
        assert!(ctx.profile.is_none());
    }
}
