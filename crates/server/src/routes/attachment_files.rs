use std::{io, path::Path};

use axum::body::Body;
use futures_util::TryStreamExt;
use muriarc_core::Attachment;
use muriarc_data::AttachmentFiles;
pub(crate) use muriarc_data::{
    AttachmentFileError, StoredAttachmentObject as StoredObject,
    VerifiedAttachmentObject as VerifiedObject,
};
use tokio_util::io::StreamReader;
use uuid::Uuid;

pub(crate) async fn write_object_with_limit(
    root: &Path,
    id: Uuid,
    body: Body,
    max_bytes: u64,
) -> Result<StoredObject, AttachmentFileError> {
    let stream = body
        .into_data_stream()
        .map_err(|error| io::Error::other(error.to_string()));
    AttachmentFiles::with_limit(root, max_bytes)
        .write_reader(id, StreamReader::new(stream))
        .await
}

pub(crate) async fn remove_installed_object(
    root: &Path,
    object: &StoredObject,
) -> Result<(), AttachmentFileError> {
    AttachmentFiles::new(root)
        .remove_installed_object(object)
        .await
}

pub(crate) async fn open_verified(
    root: &Path,
    attachment: &Attachment,
) -> Result<VerifiedObject, AttachmentFileError> {
    AttachmentFiles::new(root).open_verified(attachment).await
}
