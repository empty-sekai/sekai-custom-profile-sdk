//! 场景图元素到 Widget 的适配层。
//!
//! 该模块承接非 SDF 元素，
//! 将旧 `elements/*` 数据结构转换为 `Widget` 实例。

/// CardMember 元素 adapter。
pub mod card_member;
/// General 面板元素 adapter。
pub mod general;
/// 称号类元素 adapter。
pub mod honor;
/// 通用图片类元素 adapter。
pub mod simple_asset;
