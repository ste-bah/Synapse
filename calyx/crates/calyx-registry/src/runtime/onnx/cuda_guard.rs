use super::OnnxProviderPolicy;

pub(super) struct CudaDropGuard<T> {
    value: Option<T>,
    leak_on_drop: bool,
}

impl<T> CudaDropGuard<T> {
    pub(super) fn new(value: T, provider_policy: OnnxProviderPolicy) -> Self {
        Self {
            value: Some(value),
            leak_on_drop: provider_policy == OnnxProviderPolicy::CudaFailLoud,
        }
    }

    pub(super) fn as_ref(&self) -> &T {
        self.value
            .as_ref()
            .expect("CudaDropGuard value is present until into_inner")
    }

    pub(super) fn as_mut(&mut self) -> &mut T {
        self.value
            .as_mut()
            .expect("CudaDropGuard value is present until into_inner")
    }

    pub(super) fn into_inner(mut self) -> T {
        self.value
            .take()
            .expect("CudaDropGuard value is present until into_inner")
    }
}

impl<T> Drop for CudaDropGuard<T> {
    fn drop(&mut self) {
        if self.leak_on_drop
            && let Some(value) = self.value.take()
        {
            // ORT CUDA teardown can corrupt the glibc heap after either a failed
            // session commit or a successfully built lens. Keep guarded CUDA
            // owners process-resident; CPU-explicit owners still drop normally.
            std::mem::forget(value);
        }
    }
}
