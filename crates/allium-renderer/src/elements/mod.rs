//! 场景元素渲染模块。

#[cfg(feature = "skia-core")]
pub mod generals;
#[cfg(not(feature = "skia-core"))]
pub mod generals {}
#[cfg(feature = "skia-core")]
pub mod honor;
#[cfg(feature = "skia-core")]
pub mod image;
#[cfg(feature = "skia-core")]
pub mod shape;

use crate::types::*;

/// 扁平化后的渲染元素（统一不同类型以便排序）。
#[derive(Debug)]
pub enum RenderElement<'a> {
    Text(&'a TextElement),
    Shape(&'a ShapeElement),
    CardMember(&'a CardMemberElement),
    Stamp(&'a StampElement),
    Other(&'a OtherElement),
    BondsHonor(&'a BondsHonorElement),
    Honor(&'a HonorElement),
    Collection(&'a CollectionElement),
    General(&'a GeneralElement),
    StandMember(&'a StandMemberElement),
    GeneralBackground(&'a GeneralBackgroundElement),
    StoryBackground(&'a StoryBackgroundElement),
}

impl<'a> RenderElement<'a> {
    pub fn object_data(&self) -> &ObjectData {
        match self {
            Self::Text(e) => &e.object_data,
            Self::Shape(e) => &e.object_data,
            Self::CardMember(e) => &e.object_data,
            Self::Stamp(e) => &e.object_data,
            Self::Other(e) => &e.object_data,
            Self::BondsHonor(e) => &e.object_data,
            Self::Honor(e) => &e.object_data,
            Self::Collection(e) => &e.object_data,
            Self::General(e) => &e.object_data,
            Self::StandMember(e) => &e.object_data,
            Self::GeneralBackground(e) => &e.object_data,
            Self::StoryBackground(e) => &e.object_data,
        }
    }

    pub fn layer(&self) -> i32 {
        self.object_data().layer
    }

    pub fn visible(&self) -> bool {
        self.object_data().visible
    }
}

/// 从 CustomProfileCard 提取所有元素，按 layer 升序排序。
pub fn flatten_and_sort(card: &CustomProfileCard) -> Vec<RenderElement<'_>> {
    let total = card.texts.len()
        + card.shapes.len()
        + card.card_members.len()
        + card.stamps.len()
        + card.others.len()
        + card.bonds_honors.len()
        + card.honors.len()
        + card.collections.len()
        + card.generals.len()
        + card.stand_members.len()
        + card.general_backgrounds.len()
        + card.story_backgrounds.len();
    let mut elements: Vec<RenderElement<'_>> = Vec::with_capacity(total);

    for e in &card.texts {
        elements.push(RenderElement::Text(e));
    }
    for e in &card.shapes {
        elements.push(RenderElement::Shape(e));
    }
    for e in &card.card_members {
        elements.push(RenderElement::CardMember(e));
    }
    for e in &card.stamps {
        elements.push(RenderElement::Stamp(e));
    }
    for e in &card.others {
        elements.push(RenderElement::Other(e));
    }
    for e in &card.bonds_honors {
        elements.push(RenderElement::BondsHonor(e));
    }
    for e in &card.honors {
        elements.push(RenderElement::Honor(e));
    }
    for e in &card.collections {
        elements.push(RenderElement::Collection(e));
    }
    for e in &card.generals {
        elements.push(RenderElement::General(e));
    }
    for e in &card.stand_members {
        elements.push(RenderElement::StandMember(e));
    }
    for e in &card.general_backgrounds {
        elements.push(RenderElement::GeneralBackground(e));
    }
    for e in &card.story_backgrounds {
        elements.push(RenderElement::StoryBackground(e));
    }

    elements.sort_by_key(|e| e.layer());
    elements
}

#[cfg(feature = "skia-core")]
pub fn draw_element(
    canvas: &skia_safe::Canvas,
    elem: &RenderElement<'_>,
    md: &crate::masterdata::MasterData,
    assets: Option<&crate::assets::AssetStore>,
    profile: Option<&crate::profile::ProfileData>,
) {
    // 单次调用便利包装：自行构造共享上下文。批量渲染请用 draw_element_on_canvas
    // 并在循环外复用 fallback_assets / theme。
    let fallback_assets = crate::assets::AssetStore::new(1);
    let theme = crate::widgets::theme::Theme::default();
    draw_element_on_canvas(
        canvas,
        elem,
        md,
        assets,
        profile,
        &fallback_assets,
        &theme,
        crate::transform::CANVAS_WIDTH,
        crate::transform::CANVAS_HEIGHT,
    );
}

#[cfg(feature = "skia-core")]
#[allow(clippy::too_many_arguments)]
pub fn draw_element_on_canvas(
    canvas: &skia_safe::Canvas,
    elem: &RenderElement<'_>,
    md: &crate::masterdata::MasterData,
    assets: Option<&crate::assets::AssetStore>,
    profile: Option<&crate::profile::ProfileData>,
    fallback_assets: &crate::assets::AssetStore,
    theme: &crate::widgets::theme::Theme,
    canvas_width: f32,
    canvas_height: f32,
) {
    use crate::context::RenderContext;
    use crate::elements::shape::draw_shape;
    use crate::text::{draw_text, TEXT_SCALE};
    use crate::transform;
    use crate::widgets::adapters::card_member::CardMemberWidget;
    use crate::widgets::adapters::general::GeneralWidget;
    use crate::widgets::adapters::honor::{BondsHonorWidget, HonorWidget};
    use crate::widgets::adapters::simple_asset::{
        CollectionWidget, GeneralBgWidget, OtherWidget, StampWidget, StandMemberWidget,
        StoryBgWidget,
    };
    use crate::widgets::Widget;
    use skia_safe::Point;

    let obj = elem.object_data();
    let (x, y, angle, sx, sy) =
        transform::extract_transform_for_canvas(obj, canvas_width, canvas_height);

    canvas.save();
    canvas.translate(Point::new(x, y));
    if angle.abs() > 0.01 {
        canvas.rotate(angle, None);
    }
    if (sx - 1.0).abs() > 0.001 || (sy - 1.0).abs() > 0.001 {
        canvas.scale((sx, sy));
    }

    match elem {
        RenderElement::Text(e) => {
            tracing::debug!(
                x = x, y = y, angle = angle, sx = sx, sy = sy,
                text = %e.text.chars().take(20).collect::<String>(),
                "Text 元素坐标"
            );
            canvas.scale((TEXT_SCALE, TEXT_SCALE));
            draw_text(canvas, e, md);
        }
        RenderElement::Shape(e) => draw_shape(canvas, e, md, assets),
        RenderElement::CardMember(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let mut ctx = RenderContext::new(asset_store, theme).with_masterdata(md);
            if let Some(profile) = profile {
                ctx = ctx.with_profile(profile);
            }
            if let Some(widget) = CardMemberWidget::from_element(e, &ctx) {
                widget.draw(canvas, 0.0, 0.0, &ctx);
            }
        }
        RenderElement::Stamp(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let ctx = RenderContext::new(asset_store, theme).with_masterdata(md);
            StampWidget::from_element(e, &ctx).draw(canvas, 0.0, 0.0, &ctx);
        }
        RenderElement::Other(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let ctx = RenderContext::new(asset_store, theme).with_masterdata(md);
            if let Some(widget) = OtherWidget::from_element(e, &ctx) {
                widget.draw(canvas, 0.0, 0.0, &ctx);
            }
        }
        RenderElement::BondsHonor(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let ctx = RenderContext::new(asset_store, theme).with_masterdata(md);
            BondsHonorWidget::from_element(e).draw(canvas, 0.0, 0.0, &ctx);
        }
        RenderElement::Honor(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let mut ctx = RenderContext::new(asset_store, theme).with_masterdata(md);
            if let Some(profile) = profile {
                ctx = ctx.with_profile(profile);
            }
            HonorWidget::from_element(e).draw(canvas, 0.0, 0.0, &ctx);
        }
        RenderElement::Collection(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let ctx = RenderContext::new(asset_store, theme).with_masterdata(md);
            if let Some(widget) = CollectionWidget::from_element(e, &ctx) {
                widget.draw(canvas, 0.0, 0.0, &ctx);
            }
        }
        RenderElement::General(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let mut ctx = RenderContext::new(asset_store, theme).with_masterdata(md);
            if let Some(profile) = profile {
                ctx = ctx.with_profile(profile);
            }
            GeneralWidget::from_element(e).draw(canvas, 0.0, 0.0, &ctx);
        }
        RenderElement::StandMember(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let ctx = RenderContext::new(asset_store, theme).with_masterdata(md);
            if let Some(widget) = StandMemberWidget::from_element(e, &ctx) {
                widget.draw(canvas, 0.0, 0.0, &ctx);
            }
        }
        RenderElement::GeneralBackground(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let ctx = RenderContext::new(asset_store, theme).with_masterdata(md);
            if let Some(widget) = GeneralBgWidget::from_element(e, &ctx) {
                widget.draw(canvas, 0.0, 0.0, &ctx);
            }
        }
        RenderElement::StoryBackground(e) => {
            let asset_store = assets.unwrap_or(fallback_assets);
            let ctx = RenderContext::new(asset_store, theme).with_masterdata(md);
            if let Some(widget) = StoryBgWidget::from_element(e, &ctx) {
                widget.draw(canvas, 0.0, 0.0, &ctx);
            }
        }
    }

    canvas.restore();
}
