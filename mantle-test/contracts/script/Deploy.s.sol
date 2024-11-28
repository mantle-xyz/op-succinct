pragma solidity ^0.8.20;

import {Script} from "forge-std/Script.sol";
import "forge-std/Test.sol";
import {SP1VerifierGateway} from "@sp1-contracts/SP1VerifierGateway.sol";
import {SP1Verifier} from "@sp1-contracts/v3.0.0/SP1VerifierGroth16.sol";

import {mantle} from "../src/mantle.sol";

contract MantleValidatorDeploy is Script {
    function run() public {
        uint256 deployKey = uint256(vm.envBytes32("ETH_WALLET_PRIVATE_KEY"));
        vm.startBroadcast(deployKey);

        // Deploy SP1VerifierGateway
        bytes32 CREATE2_SALT = bytes32(uint256(0x0));
        address OWNER = vm.addr(deployKey); // Use deployer address as owner
        address gateway = address(new SP1VerifierGateway{salt: CREATE2_SALT}(OWNER));
        console2.log("SP1VerifierGateway deployed at: ", gateway);

        // Deploy SP1VerifierV3
        bytes32 CREATE2_SALT_V3 = bytes32(uint256(0x1));
        address verifier = address(new SP1Verifier{salt: CREATE2_SALT_V3}());
        console2.log("SP1VerifierV3 deployed at: ", verifier);

        // Add SP1VerifierV3 to SP1VerifierGateway
        SP1VerifierGateway gatewayV3 = SP1VerifierGateway(gateway);
        gatewayV3.addRoute(verifier);
        console2.log("SP1VerifierV3 added to SP1VerifierGateway");

        // Deploy MantleValidator
        bytes32 mantleProgramVKey = bytes32(vm.envBytes32("MANTLE_PROGRAM_VKEY"));
        address MANTLE_VALIDATOR = address(new mantle(verifier, mantleProgramVKey));
        console2.log("MantleValidator deployed at: ", MANTLE_VALIDATOR);
    }
}
