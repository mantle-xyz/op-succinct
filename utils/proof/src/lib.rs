use std::sync::Arc;

use op_succinct_elfs::RANGE_ELF_EMBEDDED;
use op_succinct_ethereum_host_utils::host::SingleChainOPSuccinctHost;
use op_succinct_host_utils::fetcher::OPSuccinctDataFetcher;

/// Get the range ELF depending on the feature flag.
pub fn get_range_elf_embedded() -> &'static [u8] {
    RANGE_ELF_EMBEDDED
}

/// Initialize the default (ETH-DA) host.
pub fn initialize_host(fetcher: Arc<OPSuccinctDataFetcher>) -> Arc<SingleChainOPSuccinctHost> {
    tracing::info!("Initializing host with Ethereum DA");
    Arc::new(SingleChainOPSuccinctHost::new(fetcher))
}
