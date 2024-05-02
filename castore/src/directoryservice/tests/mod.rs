//! This contains test scenarios that a given [DirectoryService] needs to pass.
//! We use [rstest] and [rstest_reuse] to provide all services we want to test
//! against, and then apply this template to all test functions.

use futures::StreamExt;
use rstest::*;
use rstest_reuse::{self, *};

use super::DirectoryService;
use crate::directoryservice;
use crate::{
    fixtures::{DIRECTORY_A, DIRECTORY_B, DIRECTORY_C},
    proto::{self, Directory},
};

mod utils;
use self::utils::make_grpc_directory_service_client;

// TODO: add tests doing individual puts of a closure, then doing a get_recursive
// (and figure out semantics if necessary)

/// This produces a template, which will be applied to all individual test functions.
/// See https://github.com/la10736/rstest/issues/130#issuecomment-968864832
#[template]
#[rstest]
#[case::grpc(make_grpc_directory_service_client().await)]
#[case::memory(directoryservice::from_addr("memory://").await.unwrap())]
#[case::sled(directoryservice::from_addr("sled://").await.unwrap())]
#[cfg_attr(all(feature = "cloud", feature = "integration"), case::bigtable(directoryservice::from_addr("bigtable://instance-1?project_id=project-1&table_name=table-1&family_name=cf1").await.unwrap()))]
pub fn directory_services(#[case] directory_service: impl DirectoryService) {}

/// Ensures asking for a directory that doesn't exist returns a Ok(None).
#[apply(directory_services)]
#[tokio::test]
async fn test_non_exist(directory_service: impl DirectoryService) {
    let resp = directory_service.get(&DIRECTORY_A.digest()).await;
    assert!(resp.unwrap().is_none())
}

/// Putting a single directory into the store, and then getting it out both via
/// `.get[_recursive]` should work.
#[apply(directory_services)]
#[tokio::test]
async fn put_get(directory_service: impl DirectoryService) {
    // Insert a Directory.
    let digest = directory_service.put(DIRECTORY_A.clone()).await.unwrap();
    assert_eq!(DIRECTORY_A.digest(), digest, "returned digest must match");

    // single get
    assert_eq!(
        Some(DIRECTORY_A.clone()),
        directory_service.get(&DIRECTORY_A.digest()).await.unwrap()
    );

    // recursive get
    assert_eq!(
        vec![Ok(DIRECTORY_A.clone())],
        directory_service
            .get_recursive(&DIRECTORY_A.digest())
            .collect::<Vec<_>>()
            .await
    );
}

/// Putting a directory closure should work, and it should be possible to get
/// back the root node both via .get[_recursive]. We don't check `.get` for the
/// leaf node is possible, as it's Ok for stores to not support that.
#[apply(directory_services)]
#[tokio::test]
async fn put_get_multiple_success(directory_service: impl DirectoryService) {
    // Insert a Directory closure.
    let mut handle = directory_service.put_multiple_start();
    handle.put(DIRECTORY_A.clone()).await.unwrap();
    handle.put(DIRECTORY_C.clone()).await.unwrap();
    let root_digest = handle.close().await.unwrap();
    assert_eq!(
        DIRECTORY_C.digest(),
        root_digest,
        "root digest should match"
    );

    // Get the root node.
    assert_eq!(
        Some(DIRECTORY_C.clone()),
        directory_service.get(&DIRECTORY_C.digest()).await.unwrap()
    );

    // Get the closure. Ensure it's sent from the root to the leaves.
    assert_eq!(
        vec![Ok(DIRECTORY_C.clone()), Ok(DIRECTORY_A.clone())],
        directory_service
            .get_recursive(&DIRECTORY_C.digest())
            .collect::<Vec<_>>()
            .await
    )
}

/// Puts a directory closure, but simulates a dumb client not deduplicating
/// its list. Ensure we still only get back a deduplicated list.
#[apply(directory_services)]
#[tokio::test]
async fn put_get_multiple_dedup(directory_service: impl DirectoryService) {
    // Insert a Directory closure.
    let mut handle = directory_service.put_multiple_start();
    handle.put(DIRECTORY_A.clone()).await.unwrap();
    handle.put(DIRECTORY_A.clone()).await.unwrap();
    handle.put(DIRECTORY_C.clone()).await.unwrap();
    let root_digest = handle.close().await.unwrap();
    assert_eq!(
        DIRECTORY_C.digest(),
        root_digest,
        "root digest should match"
    );

    // Ensure the returned closure only contains `DIRECTORY_A` once.
    assert_eq!(
        vec![Ok(DIRECTORY_C.clone()), Ok(DIRECTORY_A.clone())],
        directory_service
            .get_recursive(&DIRECTORY_C.digest())
            .collect::<Vec<_>>()
            .await
    )
}

/// Uploading A, then C (referring to A twice), then B (itself referring to A) should fail during close,
/// as B itself would be left unconnected.
#[apply(directory_services)]
#[tokio::test]
async fn upload_reject_unconnected(directory_service: impl DirectoryService) {
    let mut handle = directory_service.put_multiple_start();

    handle.put(DIRECTORY_A.clone()).await.unwrap();
    handle.put(DIRECTORY_C.clone()).await.unwrap();
    handle.put(DIRECTORY_B.clone()).await.unwrap();

    assert!(
        handle.close().await.is_err(),
        "closing handle should fail, as B would be left unconnected"
    );
}

/// Uploading a directory that refers to another directory not yet uploaded
/// should fail.
#[apply(directory_services)]
#[tokio::test]
async fn upload_reject_dangling_pointer(directory_service: impl DirectoryService) {
    let mut handle = directory_service.put_multiple_start();

    // We insert DIRECTORY_A on its own, to ensure the check runs for the
    // individual put_multiple session, not across the global DirectoryService
    // contents.
    directory_service.put(DIRECTORY_A.clone()).await.unwrap();

    // DIRECTORY_B refers to DIRECTORY_A, which is not uploaded with this handle.
    if handle.put(DIRECTORY_B.clone()).await.is_ok() {
        assert!(
            handle.close().await.is_err(),
            "when succeeding put, close must fail"
        )
    }
}

/// Try uploading a Directory failing its internal validation, ensure it gets
/// rejected.
#[apply(directory_services)]
#[tokio::test]
async fn upload_reject_failing_validation(directory_service: impl DirectoryService) {
    let broken_directory = Directory {
        symlinks: vec![proto::SymlinkNode {
            name: "".into(), // wrong!
            target: "doesntmatter".into(),
        }],
        ..Default::default()
    };
    assert!(broken_directory.validate().is_err());

    // Try to upload via single upload.
    assert!(
        directory_service
            .put(broken_directory.clone())
            .await
            .is_err(),
        "single upload must fail"
    );

    // Try to upload via put_multiple. We're a bit more permissive here, the
    // intermediate .put() might succeed, due to client-side bursting (in the
    // case of gRPC), but then the close MUST fail.
    let mut handle = directory_service.put_multiple_start();
    if handle.put(broken_directory).await.is_ok() {
        assert!(
            handle.close().await.is_err(),
            "when succeeding put, close must fail"
        )
    }
}

/// Try uploading a Directory that refers to a previously-uploaded directory.
/// Both pass their isolated validation, but the size field in the parent is wrong.
/// This should be rejected.
#[apply(directory_services)]
#[tokio::test]
async fn upload_reject_wrong_size(directory_service: impl DirectoryService) {
    let wrong_parent_directory = Directory {
        directories: vec![proto::DirectoryNode {
            name: "foo".into(),
            digest: DIRECTORY_A.digest().into(),
            size: DIRECTORY_A.size() + 42, // wrong!
        }],
        ..Default::default()
    };

    // Make sure isolated validation itself is ok
    assert!(wrong_parent_directory.validate().is_ok());

    // Now upload both. Ensure it either fails during the second put, or during
    // the close.
    let mut handle = directory_service.put_multiple_start();
    handle.put(DIRECTORY_A.clone()).await.unwrap();
    if handle.put(wrong_parent_directory).await.is_ok() {
        assert!(
            handle.close().await.is_err(),
            "when second put succeeds, close must fail"
        )
    }
}
