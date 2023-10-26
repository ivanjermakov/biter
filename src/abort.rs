use tokio::task::JoinHandle;

pub trait EnsureAbort {
    fn ensure_abort(self) -> Self;
}

impl<T> EnsureAbort for JoinHandle<T> {
    fn ensure_abort(self) -> Self {
        self.abort();
        self
    }
}
