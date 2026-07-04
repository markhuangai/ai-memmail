#[cfg(not(test))]
const WORKER_STEP_TIMEOUT: Duration = Duration::from_secs(90);
#[cfg(test)]
const WORKER_STEP_TIMEOUT: Duration = Duration::from_millis(50);

#[cfg(not(test))]
const WORKER_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
#[cfg(test)]
const WORKER_HEARTBEAT_INTERVAL: Duration = Duration::from_millis(10);

async fn run_ai_step<T, F>(
    processing: &dyn ProcessingStore,
    message: &InboundMessage,
    action: &'static str,
    future: F,
) -> Result<T, AiError>
where
    F: std::future::Future<Output = Result<T, AiError>>,
{
    let key = message.metadata.dedupe_key();
    let mut future = Box::pin(future);
    let timeout = tokio::time::sleep(WORKER_STEP_TIMEOUT);
    tokio::pin!(timeout);
    let mut heartbeat = tokio::time::interval(WORKER_HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            result = future.as_mut() => return result,
            _ = heartbeat.tick() => {
                if let Err(error) = processing.touch_processing(&key).await {
                    return Err(AiError::Provider(format!(
                        "processing heartbeat failed during {action}: {error}"
                    )));
                }
            }
            _ = &mut timeout => {
                return Err(AiError::Provider(format!(
                    "{action} timed out after {}s",
                    WORKER_STEP_TIMEOUT.as_secs()
                )));
            }
        }
    }
}
