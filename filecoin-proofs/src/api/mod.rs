use api::responses::FCPResponseStatus;
use libc;
use sector_base::api::SectorStore;
use std::cmp;
use std::ffi::CString;
use std::mem;
use std::slice::from_raw_parts;

mod internal;
pub mod responses;
pub mod util;

type SectorAccess = *const libc::c_char;

/// This is also defined in api::internal, but we make it explicit here for API consumers.
/// How big, in bytes, is a SNARK proof?
pub const SNARK_BYTES: usize = 192;

/// Seals a sector and returns its commitments and proof.
/// Unsealed data is read from `unsealed`, sealed, then written to `sealed`.
///
/// # Arguments
///
/// * `ss_ptr`        - pointer to a boxed SectorStore
/// * `unsealed`      - access to unsealed sector to be sealed
/// * `sealed`        - access to which sealed sector should be written
/// * `prover_id`     - uniquely identifies the prover
/// * `sector_id`     - uniquely identifies a sector
#[no_mangle]
pub unsafe extern "C" fn seal(
    ss_ptr: *mut Box<SectorStore>,
    unsealed_path: SectorAccess,
    sealed_path: SectorAccess,
    prover_id: &[u8; 31],
    sector_id: &[u8; 31],
) -> *mut responses::SealResponse {
    let unsealed_path_buf = util::pbuf_from_c(unsealed_path);
    let sealed_path_buf = util::pbuf_from_c(sealed_path);

    let result = internal::seal(
        &**ss_ptr,
        &unsealed_path_buf,
        &sealed_path_buf,
        *prover_id,
        *sector_id,
    );

    let mut response: responses::SealResponse = Default::default();

    match result {
        Ok((comm_r, comm_d, snark_proof)) => {
            response.status_code = FCPResponseStatus::FCPSuccess;

            response.comm_r[..32].clone_from_slice(&comm_r[..32]);
            response.comm_d[..32].clone_from_slice(&comm_d[..32]);
            response.proof[..SNARK_BYTES].clone_from_slice(&snark_proof[..SNARK_BYTES]);
        }
        Err(err) => {
            response.status_code = FCPResponseStatus::FCPUnclassifiedError;

            let msg = CString::new(format!("{:?}", err)).unwrap();
            response.error_msg = msg.as_ptr();
            mem::forget(msg);
        }
    }

    Box::into_raw(Box::new(response))
}

/// Verifies the output of seal.
///
/// # Arguments
///
/// * `ss_ptr`    - pointer to a boxed SectorStore
/// * `comm_r`    - replica commitment
/// * `comm_d`    - data commitment
/// * `prover_id` - uniquely identifies the prover
/// * `sector_id` - uniquely identifies the sector
/// * `proof`     - the proof, generated by seal()
#[no_mangle]
pub unsafe extern "C" fn verify_seal(
    ss_ptr: *mut Box<SectorStore>,
    comm_r: &[u8; 32],
    comm_d: &[u8; 32],
    prover_id: &[u8; 31],
    sector_id: &[u8; 31],
    proof: &[u8; SNARK_BYTES],
) -> *mut responses::VerifySealResponse {
    let mut response: responses::VerifySealResponse = Default::default();

    match internal::verify_seal(&**ss_ptr, *comm_r, *comm_d, *prover_id, *sector_id, proof) {
        Ok(true) => {
            response.status_code = FCPResponseStatus::FCPSuccess;
            response.is_valid = true;
        }
        Ok(false) => {
            response.status_code = FCPResponseStatus::FCPSuccess;
            response.is_valid = false;
        }
        Err(err) => {
            response.status_code = FCPResponseStatus::FCPUnclassifiedError;

            let msg = CString::new(format!("{:?}", err)).unwrap();
            response.error_msg = msg.as_ptr();
            mem::forget(msg);
        }
    }

    Box::into_raw(Box::new(response))
}

/// Unseals a range of bytes from a sealed sector and writes the resulting raw (unpreprocessed) sector to `output path`.
/// Returns a response indicating the number of original (unsealed) bytes which were written to `output_path`.
///
/// If the requested number of bytes exceeds that available in the raw data, `get_unsealed_range` will write fewer
/// than `num_bytes` bytes to `output_path`.
///
/// # Arguments
///
/// * `ss_ptr`       - pointer to a boxed SectorStore
/// * `sealed_path`  - path of sealed sector-file
/// * `output_path`  - path where sector file's unsealed bytes should be written
/// * `start_offset` - zero-based byte offset in original, unsealed sector-file
/// * `num_bytes`    - number of bytes to unseal and get (corresponds to contents of unsealed sector-file)
/// * `prover_id`    - uniquely identifies the prover
/// * `sector_id`    - uniquely identifies the sector
#[no_mangle]
pub unsafe extern "C" fn get_unsealed_range(
    ss_ptr: *mut Box<SectorStore>,
    sealed_path: SectorAccess,
    output_path: SectorAccess,
    start_offset: u64,
    num_bytes: u64,
    prover_id: &[u8; 31],
    sector_id: &[u8; 31],
) -> *mut responses::GetUnsealedRangeResponse {
    let mut response: responses::GetUnsealedRangeResponse = Default::default();

    let sealed_path_buf = util::pbuf_from_c(sealed_path);
    let output_path_buf = util::pbuf_from_c(output_path);

    match internal::get_unsealed_range(
        &**ss_ptr,
        &sealed_path_buf,
        &output_path_buf,
        *prover_id,
        *sector_id,
        start_offset,
        num_bytes,
    ) {
        Ok(num_bytes_unsealed) => {
            if num_bytes_unsealed == num_bytes {
                response.status_code = FCPResponseStatus::FCPSuccess;
            } else {
                response.status_code = FCPResponseStatus::FCPReceiverError;

                let msg = CString::new(format!(
                    "expected to unseal {}-bytes, but unsealed {}-bytes",
                    num_bytes, num_bytes_unsealed
                ))
                .unwrap();
                response.error_msg = msg.as_ptr();
                mem::forget(msg);
            }
            response.num_bytes_written = num_bytes_unsealed;
        }
        Err(err) => {
            response.status_code = FCPResponseStatus::FCPUnclassifiedError;

            let msg = CString::new(format!("{:?}", err)).unwrap();
            response.error_msg = msg.as_ptr();
            mem::forget(msg);
        }
    }

    Box::into_raw(Box::new(response))
}

/// Unseals an entire sealed sector and writes the resulting raw (unpreprocessed) sector to `output_path`.
/// Returns a status code indicating success or failure.
///
/// # Arguments
///
/// * `ss_ptr`      - pointer to a boxed SectorStore
/// * `sealed_path` - path of sealed sector-file
/// * `output_path` - path where sector file's unsealed bytes should be written
/// * `prover_id`   - uniquely identifies the prover
/// * `sector_id`   - uniquely identifies the sector
#[no_mangle]
pub unsafe extern "C" fn get_unsealed(
    ss_ptr: *mut Box<SectorStore>,
    sealed_path: SectorAccess,
    output_path: SectorAccess,
    prover_id: &[u8; 31],
    sector_id: &[u8; 31],
) -> *mut responses::GetUnsealedResponse {
    let mut response: responses::GetUnsealedResponse = Default::default();

    // How to read: &**ss_ptr throughout:
    // ss_ptr is a pointer to a Box
    // *ss_ptr is the Box.
    // **ss_ptr is the Box's content: a SectorStore.
    // &**ss_ptr is a reference to the SectorStore.
    let sector_store = &**ss_ptr;

    let sealed_path_buf = util::pbuf_from_c(sealed_path);
    let output_path_buf = util::pbuf_from_c(output_path);
    let sector_bytes = sector_store.config().max_unsealed_bytes_per_sector();

    match internal::get_unsealed_range(
        sector_store,
        &sealed_path_buf,
        &output_path_buf,
        *prover_id,
        *sector_id,
        0,
        sector_bytes,
    ) {
        Ok(num_bytes) => {
            if num_bytes == sector_bytes {
                response.status_code = FCPResponseStatus::FCPSuccess;
            } else {
                response.status_code = FCPResponseStatus::FCPReceiverError;

                let msg = CString::new(format!(
                    "expected to unseal {}-bytes, but unsealed {}-bytes",
                    sector_bytes, num_bytes
                ))
                .unwrap();
                response.error_msg = msg.as_ptr();
                mem::forget(msg);
            }
        }
        Err(err) => {
            response.status_code = FCPResponseStatus::FCPUnclassifiedError;

            let msg = CString::new(format!("{:?}", err)).unwrap();
            response.error_msg = msg.as_ptr();
            mem::forget(msg);
        }
    }

    Box::into_raw(Box::new(response))
}

/// Generates a proof-of-spacetime for the given replica commitments.
///
/// # Arguments
///
/// * `_ss_ptr`               - pointer to a boxed SectorStore
/// * `flattened_comm_rs_ptr` - pointer to the first cell in an array containing replica commitment
///                             bytes
/// * `flattened_comm_rs_len` - number of bytes in the flattened_comm_rs_ptr array (must be a
///                             multiple of 32)
/// * `_challenge_seed`       - currently unused
#[no_mangle]
pub unsafe extern "C" fn generate_post(
    _ss_ptr: *mut Box<SectorStore>,
    flattened_comm_rs_ptr: *const u8,
    flattened_comm_rs_len: libc::size_t,
    _challenge_seed: &[u8; 32],
) -> *mut responses::GeneratePoSTResponse {
    let comm_rs = from_raw_parts(flattened_comm_rs_ptr, flattened_comm_rs_len)
        .iter()
        .step_by(32)
        .fold(Default::default(), |mut acc: Vec<[u8; 32]>, item| {
            let sliced = from_raw_parts(item, 32);
            let mut x: [u8; 32] = Default::default();
            x.copy_from_slice(&sliced[..32]);
            acc.push(x);
            acc
        });

    // if more than one comm_r was provided, pretend like the first was faulty
    let fault_idxs: Vec<u8> = vec![0]
        .into_iter()
        .take(cmp::min(1, comm_rs.len() - 1))
        .collect();

    let mut result: responses::GeneratePoSTResponse = Default::default();

    result.faults_len = fault_idxs.len();
    result.faults_ptr = fault_idxs.as_ptr();

    // tell Rust to forget about the Vec; we'll free it when we free the GeneratePoSTResult
    mem::forget(fault_idxs);

    // write some fake proof
    result.proof = [42; 192];

    Box::into_raw(Box::new(result))
}

/// Verifies that a proof-of-spacetime is valid.
///
/// # Arguments
///
/// * `_ss_ptr` - pointer to a boxed SectorStore
/// * `proof`   - a proof-of-spacetime
#[no_mangle]
pub extern "C" fn verify_post(
    _ss_ptr: *mut Box<SectorStore>,
    proof: &[u8; 192],
) -> *mut responses::VerifyPoSTResponse {
    let mut res: responses::VerifyPoSTResponse = Default::default();

    if proof[0] == 42 {
        res.is_valid = true
    } else {
        res.is_valid = false
    };

    Box::into_raw(Box::new(res))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{thread_rng, Rng};
    use std::thread;

    use sector_base::api::disk_backed_storage::{
        init_new_proof_test_sector_store, init_new_sector_store, init_new_test_sector_store,
        ConfiguredStore,
    };
    use sector_base::api::responses::{
        destroy_new_sealed_sector_access_response, destroy_new_staging_sector_access_response,
        destroy_read_raw_response, destroy_write_and_preprocess_response,
    };
    use sector_base::api::{
        new_sealed_sector_access, new_staging_sector_access, read_raw, write_and_preprocess,
        SectorStore,
    };

    use sector_base::api::responses::SBResponseStatus;
    use std::ffi::CString;
    use std::fs::{create_dir_all, File};
    use std::io::Read;
    use tempfile;

    fn rust_str_to_c_str(s: &str) -> *const libc::c_char {
        CString::new(s).unwrap().into_raw()
    }

    fn create_storage(cs: &ConfiguredStore) -> *mut Box<SectorStore> {
        let staging_path = tempfile::tempdir().unwrap().path().to_owned();
        let sealed_path = tempfile::tempdir().unwrap().path().to_owned();

        create_dir_all(&staging_path).expect("failed to create staging dir");
        create_dir_all(&sealed_path).expect("failed to create sealed dir");

        let s1 = rust_str_to_c_str(&staging_path.to_str().unwrap().to_owned());
        let s2 = rust_str_to_c_str(&sealed_path.to_str().unwrap().to_owned());

        match cs {
            ConfiguredStore::Live => unsafe { init_new_sector_store(s1, s2) },
            ConfiguredStore::Test => unsafe { init_new_test_sector_store(s1, s2) },
            ConfiguredStore::ProofTest => unsafe { init_new_proof_test_sector_store(s1, s2) },
        }
    }

    // TODO: create a way to run these super-slow-by-design tests manually.
    //    fn seal_verify_live() {
    //        seal_verify_aux(ConfiguredStore::Live, 0);
    //        seal_verify_aux(ConfiguredStore::Live, 5);
    //    }
    //
    //    fn seal_unsealed_roundtrip_live() {
    //        seal_unsealed_roundtrip_aux(ConfiguredStore::Live, 0);
    //        seal_unsealed_roundtrip_aux(ConfiguredStore::Live, 5);
    //    }
    //
    //    fn seal_unsealed_range_roundtrip_live() {
    //        seal_unsealed_range_roundtrip_aux(ConfiguredStore::Live, 0);
    //        seal_unsealed_range_roundtrip_aux(ConfiguredStore::Live, 5);
    //    }

    #[test]
    #[ignore] // Slow test – run only when compiled for release.
    fn seal_verify_test() {
        seal_verify_aux(ConfiguredStore::Test, 0);
        seal_verify_aux(ConfiguredStore::Test, 5);
    }

    #[test]
    #[ignore] // Slow test – run only when compiled for release.
    fn seal_verify_proof_test() {
        seal_verify_aux(ConfiguredStore::ProofTest, 0);
        seal_verify_aux(ConfiguredStore::ProofTest, 5);
    }

    #[test]
    #[ignore] // Slow test – run only when compiled for release.
    fn seal_unsealed_roundtrip_test() {
        seal_unsealed_roundtrip_aux(ConfiguredStore::Test, 0);
        seal_unsealed_roundtrip_aux(ConfiguredStore::Test, 5);
    }

    #[test]
    #[ignore] // Slow test – run only when compiled for release.
    fn seal_unsealed_roundtrip_proof_test() {
        seal_unsealed_roundtrip_aux(ConfiguredStore::ProofTest, 0);
        seal_unsealed_roundtrip_aux(ConfiguredStore::ProofTest, 5);
    }

    #[test]
    #[ignore] // Slow test – run only when compiled for release.
    fn seal_unsealed_range_roundtrip_test() {
        seal_unsealed_range_roundtrip_aux(ConfiguredStore::Test, 0);
        seal_unsealed_range_roundtrip_aux(ConfiguredStore::Test, 5);
    }

    #[test]
    #[ignore] // Slow test – run only when compiled for release.
    fn seal_unsealed_range_roundtrip_proof_test() {
        seal_unsealed_range_roundtrip_aux(ConfiguredStore::ProofTest, 0);
        seal_unsealed_range_roundtrip_aux(ConfiguredStore::ProofTest, 5);
    }

    #[test]
    #[ignore] // Slow test – run only when compiled for release.
    fn concurrent_seal_unsealed_range_roundtrip_proof_test() {
        let threads = 5;

        let spawned = (0..threads)
            .map(|_| {
                thread::spawn(|| seal_unsealed_range_roundtrip_aux(ConfiguredStore::ProofTest, 0))
            })
            .collect::<Vec<_>>();

        for thread in spawned {
            thread.join().expect("test thread panicked");
        }
    }

    #[test]
    #[ignore] // Slow test – run only when compiled for release.
    fn write_and_preprocess_overwrites_unaligned_last_bytes() {
        write_and_preprocess_overwrites_unaligned_last_bytes_aux(ConfiguredStore::ProofTest);
    }

    #[test]
    fn generate_verify_post_roundtrip_test() {
        generate_verify_post_roundtrip_aux(ConfiguredStore::ProofTest);
    }

    #[test]
    fn max_unsealed_bytes_per_sector_checks() {
        assert_max_unsealed_bytes_per_sector(ConfiguredStore::Live, 1065353216);
        assert_max_unsealed_bytes_per_sector(ConfiguredStore::Test, 1016);
        assert_max_unsealed_bytes_per_sector(ConfiguredStore::ProofTest, 127);
    }
    fn assert_max_unsealed_bytes_per_sector(cs: ConfiguredStore, expected_bytes: u64) {
        let storage = create_storage(&cs);

        let bytes = unsafe {
            (&**storage as &SectorStore)
                .config()
                .max_unsealed_bytes_per_sector()
        };

        assert_eq!(
            bytes, expected_bytes,
            "wrong number of unsealed bytes for {:?}; got {}, expected {}",
            cs, bytes, expected_bytes
        );
    }

    fn generate_verify_post_roundtrip_aux(cs: ConfiguredStore) {
        unsafe {
            let storage = create_storage(&cs);

            let comm_rs: [u8; 32] = thread_rng().gen();
            let challenge_seed: [u8; 32] = thread_rng().gen();
            let generate_post_res = generate_post(storage, &comm_rs[0], 32, &challenge_seed);

            assert_eq!(
                FCPResponseStatus::FCPSuccess,
                (*generate_post_res).status_code,
                "generate_post failed"
            );

            let verify_post_res = verify_post(storage, &(*generate_post_res).proof);

            assert_eq!(
                FCPResponseStatus::FCPSuccess,
                (*verify_post_res).status_code,
                "error verifying PoSt"
            );
            assert_eq!(true, (*verify_post_res).is_valid, "invalid PoSt");

            responses::destroy_generate_post_response(generate_post_res);
            responses::destroy_verify_post_response(verify_post_res);
        }
    }

    fn storage_bytes(sector_store: &'static SectorStore) -> usize {
        sector_store.config().max_unsealed_bytes_per_sector() as usize
    }

    fn make_data_for_storage(
        sector_store: &'static SectorStore,
        space_for_padding: usize,
    ) -> Vec<u8> {
        make_random_bytes(storage_bytes(sector_store) - space_for_padding)
    }

    fn make_random_bytes(num_bytes_to_make: usize) -> Vec<u8> {
        let mut rng = thread_rng();
        (0..num_bytes_to_make).map(|_| rng.gen()).collect()
    }

    fn seal_verify_aux(cs: ConfiguredStore, byte_padding_amount: usize) {
        unsafe {
            let storage = create_storage(&cs);

            let new_staging_sector_access_response = new_staging_sector_access(storage);
            let new_sealed_sector_access_response = new_sealed_sector_access(storage);

            let seal_input_path = (*new_staging_sector_access_response).sector_access;
            let seal_output_path = (*new_sealed_sector_access_response).sector_access;

            let prover_id = &[2; 31];
            let sector_id = &[0; 31];

            let contents = make_data_for_storage(&**storage, byte_padding_amount);

            let write_and_preprocess_response =
                write_and_preprocess(storage, seal_input_path, &contents[0], contents.len());

            assert_eq!(
                SBResponseStatus::SBSuccess,
                (*write_and_preprocess_response).status_code,
                "write_and_preprocess failed for {:?}",
                cs
            );

            let seal_res = seal(
                storage,
                seal_input_path,
                seal_output_path,
                prover_id,
                sector_id,
            );

            assert_eq!(
                FCPResponseStatus::FCPSuccess,
                (*seal_res).status_code,
                "seal failed for {:?}",
                cs
            );

            let verify_seal_res = verify_seal(
                storage,
                &(*seal_res).comm_r,
                &(*seal_res).comm_d,
                prover_id,
                sector_id,
                &(*seal_res).proof,
            );

            assert_eq!(
                FCPResponseStatus::FCPSuccess,
                (*verify_seal_res).status_code,
                "verification failed for {:?}",
                cs
            );

            // FIXME: This test will not pass until we actually make use of the commtiments in ZigZag
            // that will be implemented in https://github.com/filecoin-project/rust-proofs/issues/145
            //        let bad_verify = unsafe {
            //            verify_seal(
            //                &result[32],
            //                &result[0],
            //                &prover_id[0],
            //                &challenge_seed[0],
            //                &result[64],
            //            )
            //        };
            // assert_eq!(20, bad_verify);

            responses::destroy_seal_response(seal_res);
            responses::destroy_verify_seal_response(verify_seal_res);
            destroy_new_staging_sector_access_response(new_staging_sector_access_response);
            destroy_new_sealed_sector_access_response(new_sealed_sector_access_response);
            destroy_write_and_preprocess_response(write_and_preprocess_response);
        }
    }

    fn write_and_preprocess_overwrites_unaligned_last_bytes_aux(cs: ConfiguredStore) {
        unsafe {
            let storage = create_storage(&cs);

            let new_staging_sector_access_response_a = new_staging_sector_access(storage);
            let new_staging_sector_access_response_b = new_staging_sector_access(storage);
            let new_sealed_sector_access_response = new_sealed_sector_access(storage);

            let seal_input_path = (*new_staging_sector_access_response_a).sector_access;
            let get_unsealed_range_output_path =
                (*new_staging_sector_access_response_b).sector_access;
            let seal_output_path = (*new_sealed_sector_access_response).sector_access;

            let prover_id = &[2; 31];
            let sector_id = &[0; 31];

            // The minimal reproduction for the bug this regression test checks is to write
            // 32 bytes, then 95 bytes.
            // The bytes must sum to 127, since that is the required unsealed sector size.
            // With suitable bytes (.e.g all 255),the bug always occurs when the first chunk is >= 32.
            // It never occurs when the first chunk is < 32.
            // The root problem was that write_and_preprocess was opening in append mode, so seeking backward
            // to overwrite the last, incomplete byte, was not happening.
            let contents_a = [255; 32];
            let contents_b = [255; 95];

            let write_and_preprocess_response_a =
                write_and_preprocess(storage, seal_input_path, &contents_a[0], contents_a.len());

            assert_eq!(
                SBResponseStatus::SBSuccess,
                (*write_and_preprocess_response_a).status_code,
                "write_and_preprocess failed for {:?}",
                cs
            );

            assert_eq!(
                contents_a.len() as u64,
                (*write_and_preprocess_response_a).num_bytes_written,
                "unexpected number of bytes written {:?}",
                cs
            );

            let write_and_preprocess_response_b =
                write_and_preprocess(storage, seal_input_path, &contents_b[0], contents_b.len());

            assert_eq!(
                SBResponseStatus::SBSuccess,
                (*write_and_preprocess_response_b).status_code,
                "write_and_preprocess failed for {:?}",
                cs
            );

            assert_eq!(
                contents_b.len() as u64,
                (*write_and_preprocess_response_b).num_bytes_written,
                "unexpected number of bytes written {:?}",
                cs
            );

            {
                let mut file = File::open(util::pbuf_from_c(seal_input_path)).unwrap();
                let mut buf = Vec::new();
                file.read_to_end(&mut buf).unwrap();

                println!("wrote ({}): {:?}", buf.len(), buf);
            }

            let seal_response = seal(
                storage,
                seal_input_path,
                seal_output_path,
                prover_id,
                sector_id,
            );

            assert_eq!(
                FCPResponseStatus::FCPSuccess,
                (*seal_response).status_code,
                "seal failed for {:?}",
                cs
            );

            let get_unsealed_response = get_unsealed(
                storage,
                seal_output_path,
                get_unsealed_range_output_path,
                prover_id,
                sector_id,
            );

            assert_eq!(
                FCPResponseStatus::FCPSuccess,
                (*get_unsealed_response).status_code,
                "get_unsealed failed for {:?}",
                cs
            );

            let mut file = File::open(util::pbuf_from_c(get_unsealed_range_output_path)).unwrap();
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).unwrap();

            assert_eq!(
                contents_a.len() + contents_b.len(),
                buf.len(),
                "length of original and unsealed contents differed for {:?}",
                cs
            );

            assert_eq!(
                contents_a[..],
                buf[0..contents_a.len()],
                "original and unsealed contents differed for {:?}",
                cs
            );

            assert_eq!(
                contents_b[..],
                buf[contents_a.len()..contents_a.len() + contents_b.len()],
                "original and unsealed contents differed for {:?}",
                cs
            );

            // order doesn't matter here - just make sure we free so that tests don't leak
            responses::destroy_seal_response(seal_response);
            responses::destroy_get_unsealed_response(get_unsealed_response);
            destroy_new_sealed_sector_access_response(new_sealed_sector_access_response);
            destroy_new_staging_sector_access_response(new_staging_sector_access_response_a);
            destroy_new_staging_sector_access_response(new_staging_sector_access_response_b);
            destroy_write_and_preprocess_response(write_and_preprocess_response_a);
            destroy_write_and_preprocess_response(write_and_preprocess_response_b);
        }
    }

    fn seal_unsealed_roundtrip_aux(cs: ConfiguredStore, byte_padding_amount: usize) {
        unsafe {
            let storage = create_storage(&cs);

            let new_staging_sector_access_response_a = new_staging_sector_access(storage);
            let new_staging_sector_access_response_b = new_staging_sector_access(storage);
            let new_sealed_sector_access_response = new_sealed_sector_access(storage);

            let seal_input_path = (*new_staging_sector_access_response_a).sector_access;
            let get_unsealed_output_path = (*new_staging_sector_access_response_b).sector_access;
            let seal_output_path = (*new_sealed_sector_access_response).sector_access;

            let prover_id = &[2; 31];
            let sector_id = &[0; 31];

            let contents = make_data_for_storage(&**storage, byte_padding_amount);

            let write_and_preprocess_response =
                write_and_preprocess(storage, seal_input_path, &contents[0], contents.len());

            assert_eq!(
                SBResponseStatus::SBSuccess,
                (*write_and_preprocess_response).status_code,
                "write_and_preprocess failed for {:?}",
                cs
            );

            let seal_response = seal(
                storage,
                seal_input_path,
                seal_output_path,
                prover_id,
                sector_id,
            );

            assert_eq!(
                FCPResponseStatus::FCPSuccess,
                (*seal_response).status_code,
                "seal failed for {:?}",
                cs
            );

            let get_unsealed_response = get_unsealed(
                storage,
                seal_output_path,
                get_unsealed_output_path,
                prover_id,
                sector_id,
            );

            assert_eq!(
                FCPResponseStatus::FCPSuccess,
                (*get_unsealed_response).status_code,
                "get_unsealed failed for {:?}",
                cs
            );

            let mut file = File::open(util::pbuf_from_c(get_unsealed_output_path)).unwrap();
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).unwrap();

            {
                let read_unsealed_response =
                    read_raw(storage, get_unsealed_output_path, 0, buf.len() as u64);

                assert_eq!(
                    (*read_unsealed_response).status_code,
                    SBResponseStatus::SBSuccess
                );

                let read_unsealed_data = from_raw_parts(
                    (*read_unsealed_response).data_ptr,
                    (*read_unsealed_response).data_len,
                );
                assert_eq!(&buf, &read_unsealed_data);
                destroy_read_raw_response(read_unsealed_response);
            }

            {
                let read_unsealed_response =
                    read_raw(storage, get_unsealed_output_path, 1, buf.len() as u64 - 2);

                assert_eq!(
                    (*read_unsealed_response).status_code,
                    SBResponseStatus::SBSuccess
                );

                let read_unsealed_data = from_raw_parts(
                    (*read_unsealed_response).data_ptr,
                    (*read_unsealed_response).data_len,
                );
                assert_eq!(&buf[1..buf.len() - 1], &read_unsealed_data[..]);
                destroy_read_raw_response(read_unsealed_response);
            }

            assert_eq!(
                contents.len(),
                buf.len() - byte_padding_amount,
                "length of original and unsealed contents differed for {:?}",
                cs
            );

            assert_eq!(
                contents[..],
                buf[0..contents.len()],
                "original and unsealed contents differed for {:?}",
                cs
            );

            // order doesn't matter here - just make sure we free so that tests don't leak
            responses::destroy_seal_response(seal_response);
            responses::destroy_get_unsealed_response(get_unsealed_response);
            destroy_new_sealed_sector_access_response(new_sealed_sector_access_response);
            destroy_new_staging_sector_access_response(new_staging_sector_access_response_a);
            destroy_new_staging_sector_access_response(new_staging_sector_access_response_b);
            destroy_write_and_preprocess_response(write_and_preprocess_response);
        }
    }

    fn seal_unsealed_range_roundtrip_aux(cs: ConfiguredStore, byte_padding_amount: usize) {
        unsafe {
            let storage = create_storage(&cs);

            let new_staging_sector_access_response_a = new_staging_sector_access(storage);
            let new_staging_sector_access_response_b = new_staging_sector_access(storage);
            let new_sealed_sector_access_response = new_sealed_sector_access(storage);

            let seal_input_path = (*new_staging_sector_access_response_a).sector_access;
            let get_unsealed_range_output_path =
                (*new_staging_sector_access_response_b).sector_access;
            let seal_output_path = (*new_sealed_sector_access_response).sector_access;

            let prover_id = &[2; 31];
            let sector_id = &[0; 31];

            let contents = make_data_for_storage(&**storage, byte_padding_amount);

            let write_and_preprocess_response =
                write_and_preprocess(storage, seal_input_path, &contents[0], contents.len());

            assert_eq!(
                SBResponseStatus::SBSuccess,
                (*write_and_preprocess_response).status_code,
                "write_and_preprocess failed for {:?}",
                cs
            );

            let seal_response = seal(
                storage,
                seal_input_path,
                seal_output_path,
                prover_id,
                sector_id,
            );

            assert_eq!(
                FCPResponseStatus::FCPSuccess,
                (*seal_response).status_code,
                "seal failed for {:?}",
                cs
            );

            let offset = 5;
            let range_length = contents.len() as u64 - offset;
            let get_unsealed_range_response = get_unsealed_range(
                storage,
                seal_output_path,
                get_unsealed_range_output_path,
                offset,
                range_length,
                prover_id,
                sector_id,
            );

            assert_eq!(
                FCPResponseStatus::FCPSuccess,
                (*get_unsealed_range_response).status_code,
                "get_unsealed_range_response failed for {:?}",
                cs
            );
            assert_eq!(
                range_length,
                (*get_unsealed_range_response).num_bytes_written,
                "expected range length {}; got {} for {:?}",
                range_length,
                (*get_unsealed_range_response).num_bytes_written,
                cs
            );

            let mut file = File::open(util::pbuf_from_c(get_unsealed_range_output_path)).unwrap();
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).unwrap();

            assert_eq!(
                contents[(offset as usize)..],
                buf[0..(range_length as usize)],
                "original and unsealed_range contents differed for {:?}",
                cs
            );

            responses::destroy_seal_response(seal_response);
            responses::destroy_get_unsealed_range_response(get_unsealed_range_response);
            destroy_new_sealed_sector_access_response(new_sealed_sector_access_response);
            destroy_new_staging_sector_access_response(new_staging_sector_access_response_a);
            destroy_new_staging_sector_access_response(new_staging_sector_access_response_b);
        }
    }
}