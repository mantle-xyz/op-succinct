set -ex
git submodule update --init --recursive --depth 1
NAMESPACE=$1
NETWORK=`echo $NAMESPACE | awk -F '-' '{print $1}'`
TYPE=${2:-"geth"}
MANTLE_CONFIG_DIR="${MANTLE_CONFIG_DIR:-$HOME/github_work/mantle-config}"
L2_RPC="${L2_RPC:-https://op-$TYPE-$NAMESPACE.qa4.gomantle.org}"
L2_NODE_RPC="${OP_NODE_RPC:-https://op-node-$NAMESPACE.qa4.gomantle.org}"
if [ "$NETWORK" == "sepolia" ]; then
  L1_RPC="https://sepolia-geth.qa4.gomantle.org"
  L1_BEACON_RPC="https://sepolia-lighthouse.qa4.gomantle.org"
else
  L1_RPC="https://hoodi-geth1.qa4.gomantle.org"
  L1_BEACON_RPC="https://hoodi-lighthouse4.qa4.gomantle.org"
fi

CONFIG_FILE="$MANTLE_CONFIG_DIR/cicd/services/mantle-op-$TYPE/app-$NAMESPACE.yaml"

read_addr() {
  local dec padded
  dec=$(yq "$1" "$CONFIG_FILE")
  padded=$(cast --to-uint256 "$dec")
  cast to-check-sum-address "0x${padded: -40}"
}

EOA_addrowner=$(read_addr ".extraObjects[0].data.EOA_addrowner")
EOA_proposer=$(read_addr ".extraObjects[0].data.EOA_proposer")
CA_L2OutputOracleProxy=$(read_addr ".extraObjects[0].data.CA_L2OutputOracleProxy")
CA_ProxyAdmin=$(read_addr ".extraObjects[0].data.CA_ProxyAdmin")
L2CHAINID=$(yq -r ".extraObjects[0].data.CHAIN_ID" "$CONFIG_FILE")
echo "===="
echo $L2CHAINID

OUTPUT=`cast call $CA_L2OutputOracleProxy "getL2Output(uint256)(tuple(uint256,bytes32,uint256))" 0 -r $L1_RPC | awk '{print $1}' | awk -F'(' '{print $2}'`
STARTING_OUTPUT_ROOT=`cast to-hex $OUTPUT`
STARTING_TIMESTAMP=`cast call $CA_L2OutputOracleProxy "startingTimestamp()(uint256)" -r $L1_RPC | awk '{print $1}' | awk -F'(' '{print $3}'`
STARTING_BLOCK_NUMBER=`cast call $CA_L2OutputOracleProxy "startingBlockNumber()(uint256)" -r $L1_RPC | awk '{print $1}' | awk -F'(' '{print $4}'`

mkdir -p contracts/deploy-config/ethereum-$NAMESPACE
mkdir -p ./configs/$L2CHAINID

curl $L2_NODE_RPC  -X POST  -H "Content-Type: application/json"  --data '{"method":"optimism_rollupConfig","params":[],"id":1,"jsonrpc":"2.0"}' | jq .result > ./configs/$L2CHAINID/rollup.json

cat > .env <<EOF
L1_RPC='$L1_RPC'
L1_BEACON_RPC='$L1_BEACON_RPC'
L2_RPC='$L2_RPC'
L2_NODE_RPC='$L2_NODE_RPC'
ROLLUP_CONFIG_PATH='./configs/$L2CHAINID/rollup.json'
EOF

cargo run --bin config --release -- --env-file .env 2>&1 | tee config.log
RANGE_VKEY_COMMITMENT=`cat config.log |grep "Range Verification Key Hash" | awk -F":" '{print $2}'`
AGGREGATION_VKEY=`cat config.log |grep "Aggregation Verification Key Hash" | awk -F":" '{print $2}'`
ROLLUP_CONFIG_HASH=`cat config.log |grep "Rollup Config Hash" | awk -F":" '{print $2}'`

exit 0
cat > contracts/deploy-config/ethereum-$NAMESPACE/default.yaml <<EOF
chainId: $L2CHAINID

envs:
  F_WALLET_TYPE: PRIVATE_KEY # options: AWS_KMS, PRIVATE_KEY, LEDGER, MNEMONIC
  F_PRIVATE_KEY: "f1e4711d1bcbced68b190afa8b8f088d946d4659e01f2412a39a85457221905a" ##deployer
  F_MNEMONIC: "law install orient cactus history mesh exclude piano tower style survey awesome"
  F_MNEMONIC_INDEX: 0
  F_AWS_KMS_KEY_ID: ""
  F_SENDER: "0xB404b246D1BB634023663F690960a916Cf6e8727" ## deployer
  F_VERBOSE: true
  F_RPC_URL: https://sepolia-geth.qa4.gomantle.org ##

explorers: {}

config:
  aggregationVkey: "$AGGREGATION_VKEY" 
  challenger: "$EOA_addrowner" 
  executeUpgradeCall: false
  finalizationPeriod: 3600
  l2BlockTime: 2
  opSuccinctL2OutputOracleImpl: "0x0000000000000000000000000000000000000000"
  owner: "$EOA_addrowner" 
  proposer: "$EOA_proposer" 
  proxyAdmin: "$CA_ProxyAdmin" 
  rangeVkeyCommitment: "$RANGE_VKEY_COMMITMENT" 
  rollupConfigHash: "$ROLLUP_CONFIG_HASH" 
  startingBlockNumber: "$STARTING_BLOCK_NUMBER" 
  startingOutputRoot: "$STARTING_OUTPUT_ROOT" 
  startingTimestamp: "$STARTING_TIMESTAMP" 
  submissionInterval: 450
  verifier: "$VERIFIER" 
  l2OutputOracleProxy: "$CA_L2OutputOracleProxy" 
EOF
