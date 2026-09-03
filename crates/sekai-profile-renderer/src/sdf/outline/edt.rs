//! 二维欧几里得距离变换（Felzenszwalb & Huttenlocher, 2012）。
//!
//! 提供 O(n) 的精确平方欧氏距离变换：先对每列做 1D 变换，再对每行做 1D 变换。
//! 用于 SDF 生成中替代逐像素遍历轮廓段的暴力距离计算。
//!
//! 参考：P. Felzenszwalb, D. Huttenlocher,
//! "Distance Transforms of Sampled Functions", Theory of Computing, 2012.

const INF: f32 = 1e20;

/// 对一行/一列采样函数 `f` 做 1D 平方距离变换，结果写入 `d`。
///
/// `f[q]` 为位置 q 的基准值（背景像素传 0，需要排除的像素传 `INF`）。
/// `d[q]` = min_q'( f[q'] + (q - q')^2 )。
///
/// `v` / `z` 为调用方预分配的工作缓冲（长度 ≥ n+1），避免热循环堆分配。
fn dt_1d(f: &[f32], d: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let mut k: usize = 0;
    v[0] = 0;
    z[0] = -INF;
    z[1] = INF;
    for q in 1..n {
        // 计算抛物线 q 与当前最低包络抛物线 v[k] 的交点
        let mut s = intersection(f, q, v[k]);
        while s <= z[k] {
            k -= 1;
            s = intersection(f, q, v[k]);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = INF;
    }

    let mut k = 0usize;
    for q in 0..n {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let dq = q as f32 - v[k] as f32;
        d[q] = dq * dq + f[v[k]];
    }
}

/// 抛物线 q 与 vk 的交点横坐标。
#[inline]
fn intersection(f: &[f32], q: usize, vk: usize) -> f32 {
    let fq = f[q];
    let fv = f[vk];
    // (f[q] + q^2 - (f[vk] + vk^2)) / (2q - 2vk)
    ((fq + (q * q) as f32) - (fv + (vk * vk) as f32)) / (2.0 * q as f32 - 2.0 * vk as f32)
}

/// 对 `width × height` 的平方-基准场 `grid` 做二维平方距离变换（原地）。
///
/// `grid[y*width + x]` 入参为基准值（0 或 INF），返回时为到最近 0 值像素的
/// 平方欧氏距离。
pub(super) fn edt_2d_sq(grid: &mut [f32], width: usize, height: usize) {
    if width == 0 || height == 0 {
        return;
    }
    let cap = width.max(height);
    let mut f = vec![0.0f32; cap];
    let mut d = vec![0.0f32; cap];
    let mut v = vec![0usize; cap + 1];
    let mut z = vec![0.0f32; cap + 1];

    // 列变换
    for x in 0..width {
        for y in 0..height {
            f[y] = grid[y * width + x];
        }
        dt_1d(&f[..height], &mut d[..height], &mut v, &mut z);
        for y in 0..height {
            grid[y * width + x] = d[y];
        }
    }

    // 行变换
    for y in 0..height {
        let row = &mut grid[y * width..y * width + width];
        f[..width].copy_from_slice(row);
        dt_1d(&f[..width], &mut d[..width], &mut v, &mut z);
        row.copy_from_slice(&d[..width]);
    }
}

/// 由二值覆盖率位图（`inside[i]` = 该像素是否在字形内部）计算签名距离场。
///
/// 返回每像素的签名距离（单位：像素），内部为负、外部为正，与解析法符号一致。
/// 距离取 inside/outside 两次 EDT 之差的方式，边界落在像素中心之间。
pub(super) fn signed_distance_from_mask(inside: &[bool], width: usize, height: usize) -> Vec<f32> {
    let n = width * height;
    let mut outside_field = vec![0.0f32; n]; // 外部像素=0（种子），内部=INF
    let mut inside_field = vec![0.0f32; n]; // 内部像素=0（种子），外部=INF
    for i in 0..n {
        if inside[i] {
            outside_field[i] = INF;
        } else {
            inside_field[i] = INF;
        }
    }
    edt_2d_sq(&mut outside_field, width, height);
    edt_2d_sq(&mut inside_field, width, height);

    let mut out = vec![0.0f32; n];
    // 距离上界：远超 spread 的距离对最终 gray 无意义（会被 clamp 到 0/1）。
    // 用对角线长度作 cap，避免退化输入（全内/全外，某一场无种子保持 INF）
    // 让 sqrt 传出天文数字污染下游。
    let cap = ((width * width + height * height) as f32).sqrt();
    for i in 0..n {
        // 内部像素：到最近外部像素（=边界）的距离取负；
        // 外部像素：到最近内部像素（=边界）的距离取正。
        // 注意必须交叉取场——内部像素在 inside_field 中是种子（恒 0），
        // 反之亦然，取错会导致全 0。
        let dist = if inside[i] {
            -outside_field[i].max(0.0).sqrt().min(cap)
        } else {
            inside_field[i].max(0.0).sqrt().min(cap)
        };
        out[i] = dist;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dt_1d_simple() {
        // f = [0, INF, INF, 0]：到最近 0 的平方距离应为 [0,1,1,0]
        let f = [0.0, INF, INF, 0.0];
        let mut d = [0.0; 4];
        let mut v = [0usize; 5];
        let mut z = [0.0f32; 5];
        dt_1d(&f, &mut d, &mut v, &mut z);
        assert_eq!(d, [0.0, 1.0, 1.0, 0.0]);
    }

    #[test]
    fn signed_distance_half_plane() {
        // 4x1：左两格外部、右两格内部。边界在 index 1|2 之间。
        let inside = [false, false, true, true];
        let sd = signed_distance_from_mask(&inside, 4, 1);
        // 外部像素正距离、内部像素负距离，单调
        assert!(sd[0] > sd[1] && sd[1] > 0.0);
        assert!(sd[2] < 0.0 && sd[2] > sd[3]);
    }

    #[test]
    fn all_outside_saturates_to_cap() {
        // 全外部（无内部种子）：外部像素取 inside_field，该场无 0 种子保持 INF，
        // sqrt 后被 cap 截断（不再传出天文数字）。验证输出有界且为正。
        let inside = [false; 9];
        let sd = signed_distance_from_mask(&inside, 3, 3);
        let cap = ((3 * 3 + 3 * 3) as f32).sqrt();
        assert!(sd.iter().all(|&v| v == cap), "全外部应饱和到 cap={cap}");
    }
}
