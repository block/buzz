#[derive(Debug)]
enum ManagedAgentChildInner {
    Tokio(Box<tokio::process::Child>),
    #[cfg(test)]
    Std(std::process::Child),
}

#[derive(Debug)]
pub(crate) struct ManagedAgentChild {
    inner: ManagedAgentChildInner,
    pid: u32,
}

impl ManagedAgentChild {
    pub(crate) fn new(inner: tokio::process::Child) -> Result<Self, String> {
        let pid = inner
            .id()
            .ok_or_else(|| "managed child has no process id after spawn".to_string())?;
        Ok(Self {
            inner: ManagedAgentChildInner::Tokio(Box::new(inner)),
            pid,
        })
    }

    pub(crate) fn id(&self) -> u32 {
        self.pid
    }

    pub(crate) fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        match &mut self.inner {
            ManagedAgentChildInner::Tokio(child) => child.try_wait(),
            #[cfg(test)]
            ManagedAgentChildInner::Std(child) => child.try_wait(),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_std_for_test(inner: std::process::Child) -> Self {
        let pid = inner.id();
        Self {
            inner: ManagedAgentChildInner::Std(inner),
            pid,
        }
    }
}
