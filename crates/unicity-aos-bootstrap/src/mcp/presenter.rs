//! Presentation stays on the calling thread for native platform APIs.
use super::interaction;

pub(super) enum ServePresenter {
    Platform(interaction::NativePresenter),
    #[cfg(unix)]
    Socket(interaction::TrayPresenter),
}

impl interaction::Presenter for ServePresenter {
    fn present(
        &mut self,
        request: &interaction::InteractionRequest,
    ) -> Result<Option<usize>, interaction::InteractionError> {
        match self {
            Self::Platform(presenter) => presenter.present(request),
            #[cfg(unix)]
            Self::Socket(presenter) => presenter.present(request),
        }
    }
}
