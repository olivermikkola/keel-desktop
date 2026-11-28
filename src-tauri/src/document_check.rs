use std::future::Future;
use std::sync::Mutex;
use tokio::sync::watch;

#[derive(Default)]
pub struct DocumentCheckControl {
    active: Mutex<Option<(String, watch::Sender<bool>)>>,
}

impl DocumentCheckControl {
    pub fn begin(&self, id: String) -> Result<DocumentCheckRun<'_>, String> {
        let mut active = self.active.lock().expect("document check lock poisoned");
        if active.is_some() {
            return Err("A document check is already running.".into());
        }
        let (stop, receiver) = watch::channel(false);
        *active = Some((id, stop));
        Ok(DocumentCheckRun {
            control: self,
            stop: receiver,
        })
    }

    pub fn stop(&self, id: &str) -> bool {
        let active = self.active.lock().expect("document check lock poisoned");
        active
            .as_ref()
            .is_some_and(|(active_id, stop)| active_id == id && stop.send(true).is_ok())
    }
}

pub struct DocumentCheckRun<'a> {
    control: &'a DocumentCheckControl,
    stop: watch::Receiver<bool>,
}

impl DocumentCheckRun<'_> {
    pub fn is_stopped(&self) -> bool {
        *self.stop.borrow()
    }

    pub async fn request<F: Future>(&mut self, request: F) -> Option<F::Output> {
        tokio::select! {
            biased;
            _ = self.stop.changed() => None,
            result = request => Some(result),
        }
    }
}

impl Drop for DocumentCheckRun<'_> {
    fn drop(&mut self) {
        self.control
            .active
            .lock()
            .expect("document check lock poisoned")
            .take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::{pending, ready};

    #[tokio::test]
    async fn stop_targets_the_active_batch_and_releases_it() {
        let control = DocumentCheckControl::default();
        let mut batch = control.begin("first".into()).unwrap();
        assert!(control.begin("another".into()).is_err());
        assert!(!control.stop("another"));
        assert_eq!(
            batch.request(ready("first reply")).await,
            Some("first reply")
        );

        let (result, ()) = tokio::join!(batch.request(pending::<()>()), async {
            assert!(control.stop("first"));
        });
        assert_eq!(result, None);
        drop(batch);

        let mut next = control.begin("second".into()).unwrap();
        assert!(!control.stop("first"));
        assert_eq!(next.request(ready("next reply")).await, Some("next reply"));
    }

    #[tokio::test]
    async fn stop_closes_the_in_flight_completion_connection() {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
        use tokio::net::TcpListener;
        use tokio::time::{timeout, Duration};

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let control = DocumentCheckControl::default();
        let mut batch = control.begin("first".into()).unwrap();

        let server = async {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut body_length = None;
            loop {
                let mut line = String::new();
                assert_ne!(stream.read_line(&mut line).await.unwrap(), 0);
                if let Some(length) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    body_length = Some(length.trim().parse().unwrap());
                }
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; body_length.expect("JSON body needs a content length")];
            stream.read_exact(&mut body).await.unwrap();
            assert!(control.stop("first"));
            assert_eq!(stream.read(&mut [0]).await.unwrap(), 0);
        };
        let completion = keel_core::agent::complete_plain_text(
            &url,
            "example",
            "Check the document.",
            "A notice",
            160,
            None,
            true,
        );
        let (result, ()) = timeout(Duration::from_secs(5), async {
            tokio::join!(batch.request(completion), server)
        })
        .await
        .expect("the interrupted HTTP connection should close");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn a_pending_stop_wins_before_a_request_is_polled() {
        let control = DocumentCheckControl::default();
        let mut batch = control.begin("first".into()).unwrap();
        assert!(!batch.is_stopped());
        assert!(control.stop("first"));
        assert!(batch.is_stopped());
        let result: Option<()> = batch.request(async { panic!("sent after stop") }).await;
        assert_eq!(result, None);
    }
}
