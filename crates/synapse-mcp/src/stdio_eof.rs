use std::{
    io,
    pin::Pin,
    task::{Context as TaskContext, Poll},
};

use tokio::io::{AsyncRead, ReadBuf};
use tokio_util::sync::CancellationToken;

pub(crate) struct CancelOnEofRead<R> {
    inner: R,
    connection_closed_cancel: CancellationToken,
    service_cancel: CancellationToken,
    log_code: &'static str,
    mode: &'static str,
    eof_seen: bool,
}

impl<R> CancelOnEofRead<R> {
    pub(crate) const fn new(
        inner: R,
        connection_closed_cancel: CancellationToken,
        service_cancel: CancellationToken,
        log_code: &'static str,
        mode: &'static str,
    ) -> Self {
        Self {
            inner,
            connection_closed_cancel,
            service_cancel,
            log_code,
            mode,
            eof_seen: false,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for CancelOnEofRead<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before_len = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if matches!(&result, Poll::Ready(Ok(())))
            && buf.filled().len() == before_len
            && !self.eof_seen
        {
            self.eof_seen = true;
            self.connection_closed_cancel.cancel();
            self.service_cancel.cancel();
            tracing::info!(
                code = self.log_code,
                mode = self.mode,
                "stdin reached EOF; cancelling transport"
            );
        }
        result
    }
}
