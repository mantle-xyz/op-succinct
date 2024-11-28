// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {ISP1Verifier} from "@sp1-contracts/ISP1Verifier.sol";

struct MantleOutputs {
    uint64 l2BlockNumber;
}
contract mantle {
    address public verifier;

    bytes32 public mantleProgramVKey;

    constructor(address _verifier, bytes32 _mantleProgramVKey){
        verifier = _verifier;
        mantleProgramVKey = _mantleProgramVKey;
    }

    // function verifyProof(uint64 _l2BlockNumber, bytes calldata _proofBytes) public view returns (bool) {
    //     MantleOutputs memory outputs = MantleOutputs(
    //         _l2BlockNumber
    //     );
    //     ISP1Verifier(verifier).verifyProof(mantleProgramVKey, abi.encode(outputs), _proofBytes);
    //     return true;
    // }

    function verifyProof(bytes calldata publicValues, bytes calldata _proofBytes) public view returns (bool) {
        ISP1Verifier(verifier).verifyProof(mantleProgramVKey, publicValues, _proofBytes);
        return true;
    }
}
