export ETH_WALLET_PRIVATE_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
export MANTLE_RPC_URL=http://localhost:8545
export MANTLE_PROGRAM_VKEY=0x00a22920c090ab5a2ee0b2e14214b7242868fb1815bd936eaa56dd48b2fb48b7

forge script --rpc-url $MANTLE_RPC_URL --broadcast script/Deploy.s.sol --private-key $ETH_WALLET_PRIVATE_KEY


# test proof
# proofrequest_01jdpksdnhehh95c6gp05e2zh6  -> groth16