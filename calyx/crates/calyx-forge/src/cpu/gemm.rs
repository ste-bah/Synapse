use wide::{f32x8, f32x16};

use crate::Result;
use crate::cpu::guard::{check_finite, check_shape_2d};

pub const TILE_M: usize = 64;
pub const TILE_K: usize = 64;

pub fn gemm_f32(a: &[f32], b: &[f32], m: usize, k: usize, n: usize, out: &mut [f32]) -> Result<()> {
    validate_gemm_inputs(a, b, m, k, n, out)?;
    out.fill(0.0);
    if m == 0 || n == 0 {
        return Ok(());
    }

    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx512f") {
            return gemm_tiled_f32x16(a, b, m, k, n, out);
        }
    }

    gemm_tiled_f32x8(a, b, m, k, n, out)
}

fn validate_gemm_inputs(
    a: &[f32],
    b: &[f32],
    m: usize,
    k: usize,
    n: usize,
    out: &[f32],
) -> Result<()> {
    check_shape_2d(a, m, k, "gemm A")?;
    check_shape_2d(b, k, n, "gemm B")?;
    check_shape_2d(out, m, n, "gemm output")?;
    check_finite(a, "cpu.gemm")?;
    check_finite(b, "cpu.gemm")?;
    Ok(())
}

fn gemm_tiled_f32x16(
    a: &[f32],
    b: &[f32],
    m: usize,
    k: usize,
    n: usize,
    out: &mut [f32],
) -> Result<()> {
    let mut packed_a = vec![0.0; k];
    for row_tile in (0..m).step_by(TILE_M) {
        let row_end = (row_tile + TILE_M).min(m);
        for row in row_tile..row_end {
            pack_a_row(a, row, m, &mut packed_a);
            for col in 0..n {
                out[col_major(row, col, m)] = dot_packed_f32x16(&packed_a, b, col, k);
            }
        }
    }
    Ok(())
}

fn gemm_tiled_f32x8(
    a: &[f32],
    b: &[f32],
    m: usize,
    k: usize,
    n: usize,
    out: &mut [f32],
) -> Result<()> {
    let mut packed_a = vec![0.0; k];
    for row_tile in (0..m).step_by(TILE_M) {
        let row_end = (row_tile + TILE_M).min(m);
        for row in row_tile..row_end {
            pack_a_row(a, row, m, &mut packed_a);
            for col in 0..n {
                out[col_major(row, col, m)] = dot_packed_f32x8(&packed_a, b, col, k);
            }
        }
    }
    Ok(())
}

fn pack_a_row(a: &[f32], row: usize, m: usize, packed: &mut [f32]) {
    for (depth, slot) in packed.iter_mut().enumerate() {
        *slot = a[col_major(row, depth, m)];
    }
}

fn dot_packed_f32x16(a_row: &[f32], b: &[f32], col: usize, k: usize) -> f32 {
    let b_col = b_col_slice(b, col, k);
    let mut sum = 0.0;
    let mut depth_tile = 0;
    while depth_tile < k {
        let depth_end = (depth_tile + TILE_K).min(k);
        let mut depth = depth_tile;
        while depth + 16 <= depth_end {
            let a_lane: &[f32; 16] = a_row[depth..depth + 16]
                .try_into()
                .expect("16-lane packed A slice");
            let b_lane: &[f32; 16] = b_col[depth..depth + 16]
                .try_into()
                .expect("16-lane B column slice");
            // DETERMINISM: keep the AVX512 lane multiply, then reduce as two
            // explicit f32x8-compatible subtotals. A full f32x16 tree reduction
            // drifts from cuBLAS in near-zero cancellation cells.
            let products = (f32x16::from(*a_lane) * f32x16::from(*b_lane)).to_array();
            for lane_chunk in products.chunks_exact(8) {
                let mut subtotal = 0.0;
                for product in lane_chunk {
                    subtotal += *product;
                }
                sum += subtotal;
            }
            depth += 16;
        }
        while depth < depth_end {
            sum += a_row[depth] * b_col[depth];
            depth += 1;
        }
        depth_tile += TILE_K;
    }
    sum
}

fn dot_packed_f32x8(a_row: &[f32], b: &[f32], col: usize, k: usize) -> f32 {
    let b_col = b_col_slice(b, col, k);
    let mut sum = 0.0;
    let mut depth_tile = 0;
    while depth_tile < k {
        let depth_end = (depth_tile + TILE_K).min(k);
        let mut depth = depth_tile;
        while depth + 8 <= depth_end {
            let a_lane: &[f32; 8] = a_row[depth..depth + 8]
                .try_into()
                .expect("8-lane packed A slice");
            let b_lane: &[f32; 8] = b_col[depth..depth + 8]
                .try_into()
                .expect("8-lane B column slice");
            sum += (f32x8::from(*a_lane) * f32x8::from(*b_lane)).reduce_add();
            depth += 8;
        }
        while depth < depth_end {
            sum += a_row[depth] * b_col[depth];
            depth += 1;
        }
        depth_tile += TILE_K;
    }
    sum
}

fn b_col_slice(b: &[f32], col: usize, k: usize) -> &[f32] {
    let start = col_major(0, col, k);
    &b[start..start + k]
}

fn col_major(row: usize, col: usize, rows: usize) -> usize {
    col * rows + row
}
