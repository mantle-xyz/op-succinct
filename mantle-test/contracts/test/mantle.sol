// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "forge-std/Test.sol";
import {Script} from "forge-std/Script.sol";
import {SP1VerifierGateway} from "@sp1-contracts/SP1VerifierGateway.sol";
import {SP1Verifier} from "@sp1-contracts/v3.0.0/SP1VerifierGroth16.sol";
import {mantle} from "../src/mantle.sol";

contract MantleTest is Test {
    SP1VerifierGateway public gateway;
    SP1Verifier public verifier;
    mantle public validator;
    address public owner;
    bytes32 public immutable mantleProgramVKey = hex"00a22920c090ab5a2ee0b2e14214b7242868fb1815bd936eaa56dd48b2fb48b7";
    address public immutable admin = makeAddr("admin");

    function setUp() public {
        // Set up owner
        vm.startPrank(admin);

        // Deploy SP1VerifierGateway
        bytes32 CREATE2_SALT = bytes32(uint256(0x0));
        gateway = new SP1VerifierGateway{salt: CREATE2_SALT}(admin);

        // Deploy SP1VerifierV3
        bytes32 CREATE2_SALT_V3 = bytes32(uint256(0x1));
        verifier = new SP1Verifier{salt: CREATE2_SALT_V3}();

        // Add SP1VerifierV3 to SP1VerifierGateway
        gateway.addRoute(address(verifier));

        // Deploy MantleValidator
        validator = new mantle(address(verifier), mantleProgramVKey);
        console2.log("MantleValidator deployed at: ", address(validator));
        vm.stopPrank();
    }

    function testVerifyProof() public view {
        // uint64 l2BlockNumber = 71632023;
        bytes memory publicValues = hex"9704450400000000";
        // proofrequest_01jdpksdnhehh95c6gp05e2zh6
        bytes memory proof =
            hex"09069090273d0c284521a1315d6797606166a2f9b11e2f284dd527dfd6b9db023f17ff690402800d5336bbe55bfda7d0d013e29ec7291777472c1750c31d1ee80b1eb3c41719b3fd3f9a91ae696f8c9f56e6c3e0cd5788c2637b94f65605647c9da0d5ea2caf425c5e1f58fff9075fd9be58be129e0bb06d8855ba57b51d4e93757a92ad17335c5fd5b7b73c210aefcae43121bf6e5d17f667a583c92f610fcf151a2a2f24f8be855d063ce286a032082c4d5802a948e315fb7d779575e4dac3f9307cdd1a9b05aac92ec5359dcebe43ba9f05d0e46ea4848b7c4f637cca38429b51cb7b2b5187b8e5399af6c0d484d11b74219ead6ebb0b2d4e24b33bc641b71160c174";
        assert(validator.verifyProof(publicValues, proof));
    }
}
