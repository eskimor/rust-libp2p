
use thiserror::Error;

use zeroconf::error as zeroconf;

/// Errors that can happen in this crate.
#[derive(Error,Debug)]
pub enum Error {
  /// Registering the service on Avahi/Bonjour failed for some reason.
  #[error("Registering the service on Avahi/Bonjour failed.")]
  RegisterServiceFailed(#[source] zeroconf::Error),
  /// Setting a txt record can fail apparently on the Bonjour implementation.
  #[error("Setting the txt record for the MDNS service failed.")]
  SetTxtRecordFailed(#[source] zeroconf::Error),
}
