// SPDX-License-Identifier: MIT
pragma solidity ^0.8.15;

import {Script} from "forge-std/Script.sol";
import {OPSuccinctL2OutputOracle} from "../src/validity/OPSuccinctL2OutputOracle.sol";
import {Utils} from "../test/helpers/Utils.sol";
import {Proxy} from "@optimism/contracts/universal/Proxy.sol";
import {Types} from "@optimism/contracts/libraries/Types.sol";
import {console} from "forge-std/console.sol";

contract OPSuccinctUpgrader is Script, Utils {
    function run() public {
        Config memory cfg = readJson(string.concat("deploy-config/", vm.envString("NETWORK"), "/default.json"));
        
        // Use implementation address from config
        address OPSuccinctL2OutputOracleImpl = cfg.opSuccinctL2OutputOracleImpl;

        // optionally use a different key for deployment
        uint256 deployPk = vm.envOr("DEPLOY_PK", uint256(0));
        uint256 adminPk = vm.envOr("ADMIN_PK", uint256(0));

        // If deployPk is not set, use the default key.
        if (deployPk != uint256(0)) {
            vm.startBroadcast(deployPk);
        } else {
            vm.startBroadcast();
        }

        if (OPSuccinctL2OutputOracleImpl == address(0)) {
            console.log("Deploying new OPSuccinctL2OutputOracle impl");
            cfg.opSuccinctL2OutputOracleImpl = address(new OPSuccinctL2OutputOracle());
        }


        if (cfg.l2OutputOracleProxy == address(0)) {
            revert("Proxy is not set");
        }

        OPSuccinctL2OutputOracle l2OutputOracle = OPSuccinctL2OutputOracle(cfg.l2OutputOracleProxy);
        uint256 latestOutputIndex = l2OutputOracle.latestOutputIndex();
        Types.OutputProposal memory latestOutput = l2OutputOracle.getL2Output(latestOutputIndex);

        cfg.startingBlockNumber = latestOutput.l2BlockNumber;
        cfg.startingTimestamp = latestOutput.timestamp;
        cfg.startingOutputRoot = latestOutput.outputRoot;

        cfg.l2OutputOracleProxy = address(new Proxy(msg.sender));
        console.log("New proxy:", cfg.l2OutputOracleProxy);

        vm.stopBroadcast();

        if (adminPk != uint256(0)) {
            vm.startBroadcast(adminPk);
        } else {
            vm.startBroadcast();
        }

        upgradeAndInitialize(cfg);

        vm.stopBroadcast();
    }
}
