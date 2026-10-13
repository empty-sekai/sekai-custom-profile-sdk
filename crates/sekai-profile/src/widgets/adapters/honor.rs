//! 称号类元素 adapter。

use crate::context::RenderContext;
use crate::types::{BondsHonorElement, HonorElement};
use crate::widgets::Widget;

/// 普通称号元素 Widget adapter。
pub struct HonorWidget {
    honor_id: i32,
    honor_level: i32,
    full_size: bool,
}

impl HonorWidget {
    /// 从普通称号元素构建 adapter。
    pub fn from_element(elem: &HonorElement) -> Self {
        Self {
            honor_id: elem.id,
            honor_level: elem.honor_level,
            full_size: elem.full_size,
        }
    }
}

impl Widget for HonorWidget {
    fn name(&self) -> &'static str {
        "honor"
    }

    fn measure(&self, _ctx: &RenderContext<'_>) -> (f32, f32) {
        if self.full_size {
            (380.0, 80.0)
        } else {
            (180.0, 80.0)
        }
    }

    fn asset_keys(&self, ctx: &RenderContext<'_>) -> Vec<String> {
        collect_honor_keys(ctx, self.honor_id, self.honor_level, self.full_size)
    }
}

/// 羁绊称号元素 Widget adapter。
pub struct BondsHonorWidget {
    bonds_honor_id: i32,
    honor_level: i32,
    full_size: bool,
    word_id: i64,
    inverse: bool,
    use_unit_virtual_singer: bool,
}

impl BondsHonorWidget {
    /// 从羁绊称号元素构建 adapter。
    pub fn from_element(elem: &BondsHonorElement) -> Self {
        Self {
            bonds_honor_id: elem.id,
            honor_level: elem.honor_level,
            full_size: elem.full_size,
            word_id: elem.word_id,
            inverse: elem.inverse,
            use_unit_virtual_singer: elem.use_unit_virtual_singer,
        }
    }
}

impl Widget for BondsHonorWidget {
    fn name(&self) -> &'static str {
        "bonds_honor"
    }

    fn measure(&self, _ctx: &RenderContext<'_>) -> (f32, f32) {
        let _ = self.honor_level;
        if self.full_size {
            (380.0, 80.0)
        } else {
            (180.0, 80.0)
        }
    }

    fn asset_keys(&self, ctx: &RenderContext<'_>) -> Vec<String> {
        collect_bonds_honor_keys(
            ctx,
            self.bonds_honor_id,
            self.full_size,
            self.word_id,
            self.inverse,
            self.use_unit_virtual_singer,
        )
    }
}

fn collect_honor_keys(
    ctx: &RenderContext<'_>,
    honor_id: i32,
    honor_level: i32,
    full_size: bool,
) -> Vec<String> {
    let Some(masterdata) = ctx.masterdata else {
        return Vec::new();
    };
    let Some(resolved) = masterdata.resolve_honor(honor_id, honor_level) else {
        return Vec::new();
    };

    resolved
        .asset_plan(full_size)
        .resources()
        .map(|resource| resource.key.clone())
        .collect()
}

fn collect_bonds_honor_keys(
    ctx: &RenderContext<'_>,
    bonds_honor_id: i32,
    full_size: bool,
    word_id: i64,
    inverse: bool,
    use_unit_virtual_singer: bool,
) -> Vec<String> {
    let Some(masterdata) = ctx.masterdata else {
        return Vec::new();
    };
    sekai_profile_core::masterdata::bonds_honor_asset_plan(
        masterdata,
        bonds_honor_id,
        full_size,
        word_id,
        inverse,
        use_unit_virtual_singer,
    )
    .map(|plan| plan.resources().map(|key| key.key.clone()).collect())
    .unwrap_or_default()
}

#[cfg(test)]
mod tests;
