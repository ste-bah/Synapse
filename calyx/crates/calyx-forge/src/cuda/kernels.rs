pub const DISTANCE_PTX: &[u8] = include_bytes!(env!("FORGE_DISTANCE_PTX_PATH"));
pub const TOPK_PTX: &[u8] = include_bytes!(env!("FORGE_TOPK_PTX_PATH"));
pub const QUANT_PTX: &[u8] = include_bytes!(env!("FORGE_QUANT_PTX_PATH"));
pub const PACKED_QUANT_PTX: &[u8] = include_bytes!(env!("FORGE_PACKED_QUANT_PTX_PATH"));
pub const MXFP_QUANT_PTX: &[u8] = include_bytes!(env!("FORGE_MXFP_QUANT_PTX_PATH"));
pub const MXFP4_GEMM_PTX: &[u8] = include_bytes!(env!("FORGE_MXFP4_GEMM_PTX_PATH"));
pub const ASSAY_PTX: &[u8] = include_bytes!(env!("FORGE_ASSAY_PTX_PATH"));
pub const ALGORITHMIC_PTX: &[u8] = include_bytes!(env!("FORGE_ALGORITHMIC_PTX_PATH"));
pub const DISTANCE_CUBIN: &[u8] = include_bytes!(env!("FORGE_DISTANCE_CUBIN_PATH"));
pub const TOPK_CUBIN: &[u8] = include_bytes!(env!("FORGE_TOPK_CUBIN_PATH"));
pub const QUANT_CUBIN: &[u8] = include_bytes!(env!("FORGE_QUANT_CUBIN_PATH"));
pub const PACKED_QUANT_CUBIN: &[u8] = include_bytes!(env!("FORGE_PACKED_QUANT_CUBIN_PATH"));
pub const MXFP_QUANT_CUBIN: &[u8] = include_bytes!(env!("FORGE_MXFP_QUANT_CUBIN_PATH"));
pub const MXFP4_GEMM_CUBIN: &[u8] = include_bytes!(env!("FORGE_MXFP4_GEMM_CUBIN_PATH"));
pub const ASSAY_CUBIN: &[u8] = include_bytes!(env!("FORGE_ASSAY_CUBIN_PATH"));
pub const ALGORITHMIC_CUBIN: &[u8] = include_bytes!(env!("FORGE_ALGORITHMIC_CUBIN_PATH"));

pub const DISTANCE_PTX_PATH: &str = env!("FORGE_DISTANCE_PTX_PATH");
pub const TOPK_PTX_PATH: &str = env!("FORGE_TOPK_PTX_PATH");
pub const QUANT_PTX_PATH: &str = env!("FORGE_QUANT_PTX_PATH");
pub const PACKED_QUANT_PTX_PATH: &str = env!("FORGE_PACKED_QUANT_PTX_PATH");
pub const MXFP_QUANT_PTX_PATH: &str = env!("FORGE_MXFP_QUANT_PTX_PATH");
pub const MXFP4_GEMM_PTX_PATH: &str = env!("FORGE_MXFP4_GEMM_PTX_PATH");
pub const ASSAY_PTX_PATH: &str = env!("FORGE_ASSAY_PTX_PATH");
pub const ALGORITHMIC_PTX_PATH: &str = env!("FORGE_ALGORITHMIC_PTX_PATH");
pub const DISTANCE_CUBIN_PATH: &str = env!("FORGE_DISTANCE_CUBIN_PATH");
pub const TOPK_CUBIN_PATH: &str = env!("FORGE_TOPK_CUBIN_PATH");
pub const QUANT_CUBIN_PATH: &str = env!("FORGE_QUANT_CUBIN_PATH");
pub const PACKED_QUANT_CUBIN_PATH: &str = env!("FORGE_PACKED_QUANT_CUBIN_PATH");
pub const MXFP_QUANT_CUBIN_PATH: &str = env!("FORGE_MXFP_QUANT_CUBIN_PATH");
pub const MXFP4_GEMM_CUBIN_PATH: &str = env!("FORGE_MXFP4_GEMM_CUBIN_PATH");
pub const ASSAY_CUBIN_PATH: &str = env!("FORGE_ASSAY_CUBIN_PATH");
pub const ALGORITHMIC_CUBIN_PATH: &str = env!("FORGE_ALGORITHMIC_CUBIN_PATH");

#[cfg(test)]
mod tests {
    use super::*;

    const BUILD_RS: &str = include_str!("../../build.rs");

    #[test]
    fn distance_ptx_is_embedded_and_has_header() {
        println!(
            "CUDA_KERNEL distance_ptx_path={} bytes={}",
            DISTANCE_PTX_PATH,
            DISTANCE_PTX.len()
        );
        assert!(!DISTANCE_PTX.is_empty());
        assert!(DISTANCE_PTX.starts_with(b"//\n") || DISTANCE_PTX.starts_with(b".version"));
        assert!(contains_bytes(DISTANCE_PTX, b".version"));
        assert!(contains_bytes(DISTANCE_PTX, b".target sm_120"));
    }

    #[test]
    fn ptx_contains_kernel_entry_points() {
        println!(
            "CUDA_KERNEL topk_ptx_path={} bytes={}",
            TOPK_PTX_PATH,
            TOPK_PTX.len()
        );
        assert!(contains_bytes(DISTANCE_PTX, b"cosine_batch_f32"));
        assert!(contains_bytes(DISTANCE_PTX, b"normalize_rows_f32"));
        assert!(contains_bytes(DISTANCE_PTX, b"copy_dense_external_f32"));
        assert!(contains_bytes(DISTANCE_PTX, b"pool_tokens_external_f32"));
        assert!(contains_bytes(
            DISTANCE_PTX,
            b"sparse_positive_external_f32"
        ));
        assert!(contains_bytes(
            DISTANCE_PTX,
            b"colbert_compact_external_f32"
        ));
        assert!(contains_bytes(
            DISTANCE_PTX,
            b"bgem3_sparse_compact_external_f32"
        ));
        assert!(contains_bytes(DISTANCE_PTX, b"validate_f32_flags"));
        assert!(contains_bytes(DISTANCE_PTX, b"validate_f32_ranges_flags"));
        assert!(contains_bytes(TOPK_PTX, b"bitonic_topk_f32"));
        assert!(contains_bytes(QUANT_PTX, b"tq_rotate_fwht_f32"));
        assert!(contains_bytes(QUANT_PTX, b"tq_quantize_rows_f32"));
        assert!(contains_bytes(QUANT_PTX, b"tq_score_prepared_v4"));
        assert!(contains_bytes(PACKED_QUANT_PTX, b"pq_binary_encode_f32"));
        assert!(contains_bytes(PACKED_QUANT_PTX, b"pq_binary_score"));
        assert!(contains_bytes(PACKED_QUANT_PTX, b"pq_int8_score"));
        assert!(contains_bytes(MXFP_QUANT_PTX, b"mq_mxfp4_encode_f32"));
        assert!(contains_bytes(MXFP_QUANT_PTX, b"mq_mxfp8_encode_f32"));
        assert!(contains_bytes(MXFP_QUANT_PTX, b"mq_mxfp_score"));
        assert!(contains_bytes(
            MXFP4_GEMM_PTX,
            b"gemm_mxfp4_fp32_accum_kernel"
        ));
        assert!(contains_bytes(ASSAY_PTX, b"assay_dcor_stats_f64"));
        assert!(contains_bytes(
            ASSAY_PTX,
            b"assay_ksg_continuous_counts_f32"
        ));
        assert!(contains_bytes(ASSAY_PTX, b"assay_entropy_radii_f32"));
        assert!(contains_bytes(ASSAY_PTX, b"assay_mixed_ksg_counts_f32"));
        assert!(contains_bytes(ASSAY_PTX, b"assay_ccm_simplex_predict_f32"));
        assert!(contains_bytes(ASSAY_PTX, b"assay_logistic_summaries_f32"));
        assert!(contains_bytes(ASSAY_PTX, b"assay_corr_matrix_f32"));
        assert!(contains_bytes(ASSAY_PTX, b"assay_invert_symmetric_f64"));
        assert!(contains_bytes(
            ASSAY_PTX,
            b"assay_granger_lag_summaries_f32"
        ));
        assert!(contains_bytes(ASSAY_PTX, b"assay_gls_powers_f64"));
        assert!(contains_bytes(
            ASSAY_PTX,
            b"assay_gls_permutation_powers_f64"
        ));
        assert!(contains_bytes(ASSAY_PTX, b"assay_acf_slotted_f64"));
        assert!(contains_bytes(ASSAY_PTX, b"assay_cross_correlation_f32"));
        assert!(contains_bytes(ASSAY_PTX, b"assay_hawkes_exposures_f64"));
        assert!(contains_bytes(ASSAY_PTX, b"assay_hawkes_kernel_sums_f64"));
        assert!(contains_bytes(ASSAY_PTX, b"assay_hawkes_em_background_f64"));
        assert!(contains_bytes(ASSAY_PTX, b"assay_hawkes_em_triggered_f64"));
        assert!(contains_bytes(ASSAY_PTX, b"assay_hawkes_em_update_f64"));
        assert!(contains_bytes(
            ASSAY_PTX,
            b"assay_hawkes_spectral_radius_f64"
        ));
        assert!(contains_bytes(ASSAY_PTX, b"assay_linear_cka_energy_f32"));
        assert!(contains_bytes(ASSAY_PTX, b"assay_linear_cka_sketch_f32"));
        assert!(contains_bytes(ASSAY_PTX, b"assay_linear_cka_pairs_f32"));
        assert!(contains_bytes(ASSAY_PTX, b"assay_mmd_permutations_f64"));
        assert!(contains_bytes(
            ASSAY_PTX,
            b"assay_mmd_change_permutations_f64"
        ));
        assert!(contains_bytes(
            ALGORITHMIC_PTX,
            b"algorithmic_byte_features"
        ));
        assert!(contains_bytes(
            ALGORITHMIC_PTX,
            b"algorithmic_sparse_hashes"
        ));
        assert!(contains_bytes(
            ALGORITHMIC_PTX,
            b"algorithmic_token_hash_words"
        ));
    }

    #[test]
    fn cubin_fast_path_artifacts_are_embedded() {
        println!(
            "CUDA_KERNEL_CUBIN distance={} bytes={} topk={} bytes={} quant={} bytes={} packed_quant={} bytes={} mxfp_quant={} bytes={} mxfp4={} bytes={} assay={} bytes={}",
            DISTANCE_CUBIN_PATH,
            DISTANCE_CUBIN.len(),
            TOPK_CUBIN_PATH,
            TOPK_CUBIN.len(),
            QUANT_CUBIN_PATH,
            QUANT_CUBIN.len(),
            PACKED_QUANT_CUBIN_PATH,
            PACKED_QUANT_CUBIN.len(),
            MXFP_QUANT_CUBIN_PATH,
            MXFP_QUANT_CUBIN.len(),
            MXFP4_GEMM_CUBIN_PATH,
            MXFP4_GEMM_CUBIN.len(),
            ASSAY_CUBIN_PATH,
            ASSAY_CUBIN.len()
        );
        assert!(DISTANCE_CUBIN.len() > 1024);
        assert!(TOPK_CUBIN.len() > 1024);
        assert!(QUANT_CUBIN.len() > 1024);
        assert!(PACKED_QUANT_CUBIN.len() > 1024);
        assert!(MXFP_QUANT_CUBIN.len() > 1024);
        assert!(MXFP4_GEMM_CUBIN.len() > 1024);
        assert!(ASSAY_CUBIN.len() > 1024);
        assert!(ALGORITHMIC_CUBIN.len() > 1024);
    }

    #[test]
    fn env_paths_point_to_materialized_out_dir_files() {
        for path in [
            DISTANCE_PTX_PATH,
            TOPK_PTX_PATH,
            QUANT_PTX_PATH,
            PACKED_QUANT_PTX_PATH,
            MXFP_QUANT_PTX_PATH,
            DISTANCE_CUBIN_PATH,
            TOPK_CUBIN_PATH,
            QUANT_CUBIN_PATH,
            PACKED_QUANT_CUBIN_PATH,
            MXFP_QUANT_CUBIN_PATH,
            MXFP4_GEMM_PTX_PATH,
            MXFP4_GEMM_CUBIN_PATH,
            ASSAY_PTX_PATH,
            ASSAY_CUBIN_PATH,
            ALGORITHMIC_PTX_PATH,
            ALGORITHMIC_CUBIN_PATH,
        ] {
            let metadata = std::fs::metadata(path).expect("kernel artifact exists in OUT_DIR");
            println!("CUDA_KERNEL_FILE path={path} bytes={}", metadata.len());
            assert!(metadata.len() > 0);
        }
    }

    #[test]
    fn build_script_uses_explicit_deterministic_math_flags() {
        assert!(!BUILD_RS.contains("--use_fast_math"));
        assert!(BUILD_RS.contains("--ftz=false"));
        assert!(BUILD_RS.contains("--prec-div=true"));
        assert!(BUILD_RS.contains("--prec-sqrt=true"));
        assert!(BUILD_RS.contains("--fmad=false"));
    }

    fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }
}
