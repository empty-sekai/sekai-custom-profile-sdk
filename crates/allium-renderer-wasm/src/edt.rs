//! Two-dimensional Euclidean distance transform (Felzenszwalb and Huttenlocher, 2012).
//!
//! Exact O(n) squared Euclidean distance transform, evaluated by columns and rows.

const INF: f32 = 1e20;

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
#[inline]
fn intersection(f: &[f32], q: usize, vk: usize) -> f32 {
    let fq = f[q];
    let fv = f[vk];
    ((fq + (q * q) as f32) - (fv + (vk * vk) as f32)) / (2.0 * q as f32 - 2.0 * vk as f32)
}

pub fn edt_2d_sq(grid: &mut [f32], width: usize, height: usize) {
    if width == 0 || height == 0 {
        return;
    }
    let cap = width.max(height);
    let mut f = vec![0.0f32; cap];
    let mut d = vec![0.0f32; cap];
    let mut v = vec![0usize; cap + 1];
    let mut z = vec![0.0f32; cap + 1];

    for x in 0..width {
        for y in 0..height {
            f[y] = grid[y * width + x];
        }
        dt_1d(&f[..height], &mut d[..height], &mut v, &mut z);
        for y in 0..height {
            grid[y * width + x] = d[y];
        }
    }

    for y in 0..height {
        let row = &mut grid[y * width..y * width + width];
        f[..width].copy_from_slice(row);
        dt_1d(&f[..width], &mut d[..width], &mut v, &mut z);
        row.copy_from_slice(&d[..width]);
    }
}

pub fn signed_distance_from_mask(inside: &[bool], width: usize, height: usize) -> Vec<f32> {
    let n = width * height;
    let mut outside_field = vec![0.0f32; n];
    let mut inside_field = vec![0.0f32; n];
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
    let cap = ((width * width + height * height) as f32).sqrt();
    for i in 0..n {
        let dist = if inside[i] {
            -outside_field[i].max(0.0).sqrt().min(cap)
        } else {
            inside_field[i].max(0.0).sqrt().min(cap)
        };
        out[i] = dist;
    }
    out
}
