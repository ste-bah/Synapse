use rand::{Rng, seq::SliceRandom};

use calyx_core::{CalyxError, Result};

pub(crate) const M_OUT_OF_N_NUMERATOR: usize = 4;
pub(crate) const M_OUT_OF_N_DENOMINATOR: usize = 5;

pub(crate) fn m_out_of_n_size(
    n: usize,
    k: usize,
    minimum: usize,
    estimator: &str,
) -> Result<usize> {
    let m = n.saturating_mul(M_OUT_OF_N_NUMERATOR) / M_OUT_OF_N_DENOMINATOR;
    if m < minimum || k == 0 || k >= m {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "{estimator} no-replacement CI requires a distinct subsample with at least {minimum} rows and 0 < k < m; got n={n}, m={m}, k={k}, fraction={M_OUT_OF_N_NUMERATOR}/{M_OUT_OF_N_DENOMINATOR}"
        )));
    }
    Ok(m)
}

pub(crate) fn sample_without_replacement_indices<R: Rng + ?Sized>(
    n: usize,
    m: usize,
    rng: &mut R,
) -> Result<Vec<usize>> {
    if m == 0 || m > n {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "no-replacement subsample requires 0 < m <= n; got n={n}, m={m}"
        )));
    }

    let mut indices = (0..n).collect::<Vec<_>>();
    indices.shuffle(rng);
    indices.truncate(m);
    if !indices_are_distinct(&indices, n) {
        return Err(CalyxError::assay_insufficient_samples(
            "no-replacement subsample duplicate index invariant violated",
        ));
    }
    Ok(indices)
}

pub(crate) fn sample_paired_values_without_replacement<R: Rng + ?Sized>(
    columns: &[&[f32]],
    m: usize,
    rng: &mut R,
) -> Result<Vec<Vec<f32>>> {
    let n = columns.first().map_or(0, |column| column.len());
    if columns.iter().any(|column| column.len() != n) {
        return Err(CalyxError::assay_insufficient_samples(
            "paired no-replacement subsample requires equal column lengths",
        ));
    }
    let indices = sample_without_replacement_indices(n, m, rng)?;
    Ok(columns
        .iter()
        .map(|column| indices.iter().map(|&index| column[index]).collect())
        .collect())
}

fn indices_are_distinct(indices: &[usize], n: usize) -> bool {
    let mut seen = vec![false; n];
    for &index in indices {
        if index >= n || seen[index] {
            return false;
        }
        seen[index] = true;
    }
    true
}
