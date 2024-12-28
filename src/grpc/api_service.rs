use tokio::time::{timeout, Duration};
use tonic::Request;
use tonic::Response;
use tonic::Status;
use tracing::debug;

use crate::protobuf::api_service_server::ApiService;
use crate::protobuf::GetRequest;
use crate::protobuf::Response as PbResponse;
use crate::protobuf::SetRequest;
use crate::store::KeyValueStore;
use crate::raft_types::*;

/// External API service implementation providing key-value store operations.
/// This service handles client requests for getting and setting values in the distributed store.
///
/// # Responsibilities
/// - Handle key-value get operations
/// - Handle key-value set operations
/// - Ensure consistency through Raft consensus
///
/// # Protocol Safety
/// This service implements the client-facing API and should validate all inputs
/// before processing them through the Raft consensus protocol.
pub struct ApiServiceImpl {
  /// The Raft node instance for consensus operations
  raft_node: Raft,
  /// The state machine's key-value store for direct reads
  key_values: KeyValueStore,
}

impl ApiServiceImpl {
  /// Creates a new instance of the API service
  ///
  /// # Arguments
  /// * `raft_node` - The Raft node instance this service will use
  /// * `state_machine_store` - The state machine store for reading data
  pub fn new(raft_node: Raft, key_values: KeyValueStore) -> Self {
    ApiServiceImpl {
      raft_node,
      key_values,
    }
  }
}

#[tonic::async_trait]
impl ApiService for ApiServiceImpl {
    /// Sets a value for a given key in the distributed store
    ///
    /// # Arguments
    /// * `request` - Contains the key and value to set
    ///
    /// # Returns
    /// * `Ok(Response)` - Success response after the value is set
    /// * `Err(Status)` - Error status if the set operation fails
    async fn set(&self, request: Request<SetRequest>) -> Result<Response<PbResponse>, Status> {
      let req = request.into_inner();
      debug!("Processing set request for key: {}", req.key.clone());

      let res = self
        .raft_node
        .client_write(req.clone())
        .await
        .map_err(|e| Status::internal(format!("Failed to write to store: {}", e)))?;

      debug!("Successfully set value for key: {}", req.key);
      Ok(Response::new(res.data))
    }

    /// Gets a value for a given key from the distributed store
    ///
    /// # Arguments
    /// * `request` - Contains the key to retrieve
    ///
    /// # Returns
    /// * `Ok(Response)` - Success response containing the value
    /// * `Err(Status)` - Error status if the get operation fails
    async fn get(&self, request: Request<GetRequest>) -> Result<Response<PbResponse>, Status> {
        let req = request.into_inner();
        debug!("Processing get request for key: {}", req.key);

        // Attempt to acquire lock with 1 second timeout
        let reader =
          timeout(Duration::from_secs(1), self.key_values.read())
          .await
          .map_err(|_| Status::deadline_exceeded("Timeout acquiring read lock on DB"))?;

        let value = reader
            .get(&req.key)
            .ok_or_else(|| Status::internal(format!("Key not found: {}", req.key)))?
            .to_string();

        debug!("Successfully retrieved value for key: {}", req.key);
        Ok(Response::new(PbResponse { value: Some(value) }))
    }
}
