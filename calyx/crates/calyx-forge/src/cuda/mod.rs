pub mod algorithmic;
pub mod assay;
pub mod context;
pub mod distance;
pub mod gemm;
pub mod green_context;
pub mod grouped_gemm;
pub mod kernels;
pub mod postprocess;
pub mod profile;
pub mod quant;
pub mod ragged_gemm;
pub mod resident;
pub mod topk;
mod validate;

use crate::{Backend, CUDA_EXACT_TOPK_MAX_K, DeviceInfo, ForgeError, KnnBatch, KnnMetric, Result};

pub use crate::mxfp4;
pub use algorithmic::{
    ALGORITHMIC_SPARSE_MAX_TOKEN_BYTES, ALGORITHMIC_TOKEN_HASH_MAX_TOKEN_BYTES,
    CudaAlgorithmicContext, CudaAlgorithmicStats, CudaByteFeatureRaw, CudaByteRaggedBatch,
};
pub use assay::{
    CUDA_GRANGER_STATUS_INVALID_LAG, CUDA_GRANGER_STATUS_NONFINITE, CUDA_GRANGER_STATUS_OK,
    CUDA_GRANGER_STATUS_RANK_DEFICIENT, CudaAutocorrelationSums, CudaCcmPredictions,
    CudaCorrelationPrecision, CudaCrossCorrelationBatch, CudaDcorResult, CudaGrangerLagBatch,
    CudaGrangerLagSummary, CudaHawkesFit, CudaHsicResult, CudaKsgContinuousCounts,
    CudaLinearCkaPairEstimates, CudaLogisticConfig, CudaLogisticDataset, CudaLogisticSplits,
    CudaLogisticSummaries, CudaMixedKsgCounts, CudaMmdChangePointResult, CudaMmdResult,
    CudaPeriodogramBatch, autocorrelation_sums_host, ccm_simplex_predictions_host,
    correlation_precision_host, cross_correlation_batch_host, dcor_1d_host, entropy_radii_host,
    gaussian_mmd_host, granger_lag_summaries_host, hawkes_em_host, hsic_1d_host,
    ksg_continuous_counts_host, linear_cka_pair_estimates_host, logistic_summaries_host,
    mixed_ksg_counts_host, mmd_change_point_host, periodogram_batch_host,
};
pub use context::{CudaContext, init_cuda, query_device_info};
pub use distance::{
    cosine_batch_gpu, dot_batch_gpu, l2_batch_gpu, normalize_rows_gpu, paired_cosine_gpu,
    paired_cosine_host,
};
pub use gemm::{
    bench_gemm_cublas, bench_gemm_reference_cublas, gemm_cublas, gemm_mxfp4_fp32_accum,
    gemm_mxfp8_fp32_accum, probe_allocation,
};
pub use green_context::CudaGreenContextStream;
pub use grouped_gemm::{
    AbsentSlotSentinel, GemmProblem, GroupedGemmExecutionMode, GroupedGemmPlan,
    build_grouped_gemm_plan, execute_grouped_gemm, execute_grouped_gemm_strict,
    read_grouped_gemm_output,
};
pub use postprocess::{
    CudaDenseTokenPostprocess, CudaMultiRows, CudaPostprocessPooling, CudaSparseRows,
    bgem3_colbert_tokens_from_external_f32, bgem3_sparse_from_external_f32,
    colbert_tokens_from_external_f32, dense_2d_from_external_f32, dense_tokens_from_external_f32,
    sparse_positive_from_external_f32,
};
pub use profile::{ProfilePairwiseCudaStats, pairwise_euclidean_gram_tiled_host};
pub use quant::{
    BINARY_CUDA_MIN_ELEMENTS, CudaBinaryBatch, CudaBinaryScores, CudaInt8Batch, CudaMxFpBatch,
    CudaQuantContext, CudaQuantScores, CudaQuantStats, CudaTurboQuantBatch, CudaTurboQuantScores,
    INT8_CUDA_MIN_ELEMENTS, MXFP4_CUDA_MIN_ELEMENTS, MXFP8_CUDA_MIN_ELEMENTS, QuantDispatch,
    TURBOQUANT_CUDA_MIN_ELEMENTS, binary_dispatch, int8_dispatch, mxfp4_dispatch, mxfp8_dispatch,
    turboquant_dispatch,
};
pub use ragged_gemm::{
    RaggedBatch, build_ragged_batch, build_ragged_batch_from_slabs, extract_ragged_results,
    try_extract_ragged_results,
};
pub use resident::{DeviceCandidateBlock, cosine_resident_host, upload_candidate_block};
pub use topk::topk_gpu;

#[derive(Clone, Debug)]
pub struct CudaBackend {
    ctx: CudaContext,
}

impl CudaBackend {
    pub fn new() -> Result<Self> {
        init_cuda(0, false).map(|ctx| Self { ctx })
    }

    pub fn with_context(ctx: CudaContext) -> Self {
        Self { ctx }
    }

    pub fn context(&self) -> &CudaContext {
        &self.ctx
    }

    pub fn grouped_gemm(&self, plan: &mut GroupedGemmPlan) -> Result<()> {
        grouped_gemm::execute_grouped_gemm(&self.ctx, plan)
    }

    pub fn grouped_gemm_strict(&self, plan: &mut GroupedGemmPlan) -> Result<()> {
        grouped_gemm::execute_grouped_gemm_strict(&self.ctx, plan)
    }
}

impl Backend for CudaBackend {
    fn gemm(
        &self,
        a: &[f32],
        b: &[f32],
        m: usize,
        k: usize,
        n: usize,
        out: &mut [f32],
    ) -> Result<()> {
        gemm::gemm_host(&self.ctx, a, b, m, k, n, out)
    }

    fn cosine(&self, a: &[f32], b: &[f32], dim: usize, out: &mut [f32]) -> Result<()> {
        distance::cosine_host(&self.ctx, a, b, dim, out)
    }

    fn dot(&self, a: &[f32], b: &[f32], dim: usize, out: &mut [f32]) -> Result<()> {
        distance::dot_host(&self.ctx, a, b, dim, out)
    }

    fn l2(&self, a: &[f32], b: &[f32], dim: usize, out: &mut [f32]) -> Result<()> {
        distance::l2_host(&self.ctx, a, b, dim, out)
    }

    fn normalize(&self, vecs: &mut [f32], dim: usize) -> Result<()> {
        distance::normalize_host(&self.ctx, vecs, dim)
    }

    fn topk(&self, scores: &[f32], k: usize) -> Result<Vec<(usize, f32)>> {
        topk::topk_host(&self.ctx, scores, k)
    }

    fn knn(
        &self,
        queries: &[f32],
        candidates: &[f32],
        query_count: usize,
        dim: usize,
        k: usize,
        metric: KnnMetric,
    ) -> Result<KnnBatch> {
        knn_cuda(&self.ctx, queries, candidates, query_count, dim, k, metric)
    }

    fn paired_cosine(
        &self,
        left: &[f32],
        right: &[f32],
        pair_count: usize,
        dim: usize,
        out: &mut [f32],
    ) -> Result<()> {
        distance::paired_cosine_host(&self.ctx, left, right, pair_count, dim, out)
    }

    fn device_info(&self) -> DeviceInfo {
        query_device_info(&self.ctx)
    }
}

fn knn_cuda(
    ctx: &CudaContext,
    queries: &[f32],
    candidates: &[f32],
    query_count: usize,
    dim: usize,
    k: usize,
    metric: KnnMetric,
) -> Result<KnnBatch> {
    let candidate_count = crate::cpu::validate_knn_shape(queries, candidates, query_count, dim)?;
    let k_eff = k.min(candidate_count);
    if query_count == 0 || k_eff == 0 {
        return KnnBatch::new(
            query_count,
            0,
            candidate_count,
            metric,
            Vec::new(),
            Vec::new(),
        );
    }
    if matches!(metric, KnnMetric::Cosine | KnnMetric::Dot) && k_eff > CUDA_EXACT_TOPK_MAX_K {
        return Err(ForgeError::ShapeMismatch {
            expected: vec![CUDA_EXACT_TOPK_MAX_K],
            got: vec![k_eff],
            remediation: format!(
                "cuda knn uses exact topk and is bounded to k <= {CUDA_EXACT_TOPK_MAX_K}"
            ),
        });
    }
    let total_hits = query_count
        .checked_mul(k_eff)
        .ok_or_else(|| ForgeError::ShapeMismatch {
            expected: vec![query_count, k_eff],
            got: vec![usize::MAX],
            remediation: "cuda knn output shape overflows usize".to_string(),
        })?;
    let mut indices = Vec::with_capacity(total_hits);
    let mut scores = Vec::with_capacity(total_hits);
    let stream = ctx.inner().default_stream();
    let candidates_dev = stream
        .clone_htod(candidates)
        .map_err(|err| cuda_knn_device(ctx, format!("candidate upload failed: {err}")))?;

    for query in queries.chunks_exact(dim) {
        let query_dev = stream
            .clone_htod(query)
            .map_err(|err| cuda_knn_device(ctx, format!("query upload failed: {err}")))?;
        let mut out_dev = stream
            .alloc_zeros(candidate_count)
            .map_err(|err| cuda_knn_device(ctx, format!("score allocation failed: {err}")))?;
        match metric {
            KnnMetric::Cosine => {
                distance::cosine_batch_gpu(
                    ctx,
                    &query_dev,
                    &candidates_dev,
                    dim,
                    candidate_count,
                    &mut out_dev,
                )?;
                append_ranked(
                    &mut indices,
                    &mut scores,
                    topk::topk_gpu(ctx, &out_dev, k_eff, candidate_count)?,
                );
            }
            KnnMetric::Dot => {
                distance::dot_batch_gpu(
                    ctx,
                    &query_dev,
                    &candidates_dev,
                    dim,
                    candidate_count,
                    &mut out_dev,
                )?;
                append_ranked(
                    &mut indices,
                    &mut scores,
                    topk::topk_gpu(ctx, &out_dev, k_eff, candidate_count)?,
                );
            }
            KnnMetric::L2Squared => {
                distance::l2_batch_gpu(
                    ctx,
                    &query_dev,
                    &candidates_dev,
                    dim,
                    candidate_count,
                    &mut out_dev,
                )?;
                let l2_scores =
                    distance::read_checked_device_output(ctx, "knn_l2_squared", &out_dev, false)?;
                append_ranked(&mut indices, &mut scores, rank_l2(&l2_scores, k_eff));
            }
        }
    }
    KnnBatch::new(query_count, k_eff, candidate_count, metric, indices, scores)
}

fn append_ranked(indices: &mut Vec<usize>, scores: &mut Vec<f32>, ranked: Vec<(usize, f32)>) {
    for (index, score) in ranked {
        indices.push(index);
        scores.push(score);
    }
}

fn rank_l2(scores: &[f32], k: usize) -> Vec<(usize, f32)> {
    let mut ranked = scores
        .iter()
        .copied()
        .enumerate()
        .collect::<Vec<(usize, f32)>>();
    ranked.sort_by(|left, right| {
        left.1
            .total_cmp(&right.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    ranked.truncate(k);
    ranked
}

fn cuda_knn_device(ctx: &CudaContext, detail: String) -> ForgeError {
    ForgeError::DeviceUnavailable {
        device: format!("cuda:{}", ctx.device_idx()),
        detail: format!("cuda knn {detail}"),
        remediation: "Check CUDA KNN inputs, VRAM, and device availability".to_string(),
    }
}
