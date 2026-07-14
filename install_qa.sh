set -ex
git submodule update --init --recursive --depth 1
NAMESPACE=$1
NETWORK=`echo $NAMESPACE | awk -F '-' '{print $1}'`
TYPE=${2:-"geth"}
MANTLE_CONFIG_DIR="${MANTLE_CONFIG_DIR:-$HOME/github_work/mantle-config}"
cd $MANTLE_CONFIG_DIR;git pull origin main --ff-only;cd -
L2_RPC="${L2_RPC:-https://op-$TYPE-$NAMESPACE.qa4.gomantle.org/?token=gomantle2026}"
L2_NODE_RPC="${OP_NODE_RPC:-https://op-node-$NAMESPACE.qa4.gomantle.org/?token=gomantle2026}"
if [ "$NETWORK" == "sepolia" ]; then
  L1_RPC="https://sepolia-geth.qa4.gomantle.org"
  L1_BEACON_RPC="https://sepolia-lighthouse.qa4.gomantle.org"
else
  L1_RPC="https://hoodi-geth1.qa4.gomantle.org"
  L1_BEACON_RPC="https://hoodi-lighthouse4.qa4.gomantle.org"
fi

CONFIG_FILE="$MANTLE_CONFIG_DIR/cicd/services/mantle-op-$TYPE/app-$NAMESPACE.yaml"
decode_calldata() {

  CALLDATA="$1"

  echo "== 外层 CALLDATA =="
  echo "$CALLDATA"
  echo

  # 2) 解码外层 ProxyAdmin.upgradeAndCall(address proxy, address impl, bytes data)
  echo "== 解码 upgradeAndCall(address,address,bytes) =="
  OUTER=$(cast calldata-decode "upgradeAndCall(address,address,bytes)" "$CALLDATA")
  echo "$OUTER"
  echo

  PROXY=$(echo "$OUTER"  | sed -n '1p' | awk '{print $1}')
  IMPL=$(echo  "$OUTER"  | sed -n '2p' | awk '{print $1}')
  INNER=$(echo "$OUTER"  | sed -n '3p' | awk '{print $1}')

  echo "proxy          = $PROXY"
  echo "implementation = $IMPL"
  echo "inner selector = ${INNER:0:10}"
  echo

  # 3) 解码内层 initialize(InitParams)
  #    struct InitParams {
  #      address challenger; address proposer; address owner;
  #      uint256 finalizationPeriodSeconds; uint256 l2BlockTime;
  #      bytes32 aggregationVkey; bytes32 rangeVkeyCommitment; bytes32 rollupConfigHash;
  #      bytes32 startingOutputRoot; uint256 startingBlockNumber; uint256 startingTimestamp;
  #      uint256 submissionInterval; address verifier;
  #    }
  INIT_SIG="initialize((address,address,address,uint256,uint256,bytes32,bytes32,bytes32,bytes32,uint256,uint256,uint256,address))"

  echo "== 解码内层 $INIT_SIG =="
  DECODED=$(cast calldata-decode "$INIT_SIG" "$INNER")
  echo "$DECODED"
  echo

  # 4) 逐字段标注
  FIELDS=(challenger proposer owner finalizationPeriodSeconds l2BlockTime \
          aggregationVkey rangeVkeyCommitment rollupConfigHash startingOutputRoot \
          startingBlockNumber startingTimestamp submissionInterval verifier)

  echo "== 字段核对 =="
  # cast 把 tuple 输出成一行: (v0, v1, ...)。去掉外层括号后按 ", " 拆分。
  INNER_TUPLE="${DECODED#\(}"
  INNER_TUPLE="${INNER_TUPLE%\)}"
  IFS=',' read -ra VALS <<< "$INNER_TUPLE"
  for i in "${!FIELDS[@]}"; do
    v="${VALS[$i]}"
    v="${v#"${v%%[![:space:]]*}"}"   # 去掉前导空格
    printf "%-26s = %s\n" "${FIELDS[$i]}" "$v"
  done
  
}
read_addr() {
  local dec padded
  dec=$(yq "$1" "$CONFIG_FILE")
  padded=$(cast --to-uint256 "$dec")
  cast to-check-sum-address "0x${padded: -40}"
}

EOA_addrowner=$(read_addr ".extraObjects[0].data.EOA_addrowner")
EOA_proposer=$(read_addr ".extraObjects[0].data.EOA_proposer")
EOA_deployer=$(read_addr ".extraObjects[0].data.EOA_deployer")
KEY_deployer=$(yq -r ".extraObjects[0].data.KEY_deployer" "$CONFIG_FILE")
KEY_addrowner=$(yq -r ".extraObjects[0].data.KEY_addrowner" "$CONFIG_FILE")
OP_PROPOSER_MNEMONIC=$(yq -r ".extraObjects[0].data.OP_PROPOSER_MNEMONIC" "$CONFIG_FILE")
CA_L2OutputOracleProxy=$(read_addr ".extraObjects[0].data.CA_L2OutputOracleProxy")
CA_ProxyAdmin=$(read_addr ".extraObjects[0].data.CA_ProxyAdmin")
L2CHAINID=$(yq -r ".extraObjects[0].data.CHAIN_ID" "$CONFIG_FILE")
echo "===="
echo $L2CHAINID

OUTPUT=$(cast call $CA_L2OutputOracleProxy "getL2Output(uint256)(tuple(uint256,bytes32,uint256))" 0 -r $L1_RPC | awk '{print $1}' | awk -F'(' '{print $2}')
STARTING_OUTPUT_ROOT=$(cast to-hex $OUTPUT)
STARTING_TIMESTAMP=$(cast call $CA_L2OutputOracleProxy "startingTimestamp()(uint256)" -r $L1_RPC | awk '{print $1}')
STARTING_BLOCK_NUMBER=$(cast call $CA_L2OutputOracleProxy "startingBlockNumber()(uint256)" -r $L1_RPC | awk '{print $1}')

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

cargo run --bin config --release -- --env-file .env 2>&1 | tee run.log
RANGE_VKEY_COMMITMENT=$(grep 'Range Verification Key Hash' run.log | awk '{print $NF}')
AGGREGATION_VKEY=$(grep 'Aggregation Verification Key Hash' run.log | awk '{print $NF}')
ROLLUP_CONFIG_HASH=$(grep 'Rollup Config Hash' run.log | awk '{print $NF}')

mkdir -p contracts/deploy-config/ethereum-$NAMESPACE
cat > contracts/deploy-config/ethereum-$NAMESPACE/default.yaml <<EOF
chainId: $L2CHAINID

envs:
  F_WALLET_TYPE: PRIVATE_KEY # options: AWS_KMS, PRIVATE_KEY, LEDGER, MNEMONIC
  F_PRIVATE_KEY: "$KEY_deployer"
  F_MNEMONIC: "$OP_PROPOSER_MNEMONIC"
  F_MNEMONIC_INDEX: 0
  F_AWS_KMS_KEY_ID: ""
  F_SENDER: "$EOA_deployer" ## deployer
  F_VERBOSE: true
  F_RPC_URL: $L1_RPC 

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
  startingBlockNumber: $STARTING_BLOCK_NUMBER
  startingOutputRoot: "$STARTING_OUTPUT_ROOT" 
  startingTimestamp: $STARTING_TIMESTAMP
  submissionInterval: 450
  verifier: "0xd685a80aF2d1761648e56716af4868d850Dae49B" 
  l2OutputOracleProxy: "$CA_L2OutputOracleProxy" 
EOF

cd contracts
NETWORK=ethereum-$NAMESPACE task Upgrade -- --broadcast 2>&1 | tee upgrade.log  
CALLDATA=$(grep -A1 "calldata for upgrading" "upgrade.log" | grep -oE "0x[0-9a-fA-F]+" | head -n1 | tr -d '[:space:]')
SUCCINCTIMPL=$(grep "The impl are" upgrade.log | grep -oE "0x[0-9a-fA-F]{40}"| head -n1)
set +x;decode_calldata "$CALLDATA";set -x

kubectl -n $NAMESPACE  scale sts mantle-op-proposer --replicas 0
cast send --private-key $KEY_addrowner \
  $CA_ProxyAdmin \
  "upgrade(address,address)" \
  $CA_L2OutputOracleProxy \
  $SUCCINCTIMPL \
  -r $L1_RPC

  
cast send $CA_ProxyAdmin "$CALLDATA" --rpc-url $L1_RPC --private-key $KEY_addrowner

