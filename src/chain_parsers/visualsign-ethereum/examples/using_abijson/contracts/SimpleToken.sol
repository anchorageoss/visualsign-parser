// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/**
 * @title SimpleToken
 * @dev A minimal example contract for demonstrating ABI-based visualization
 */
contract SimpleToken {
    mapping(address => uint256) public balances;

    /// @dev Mint new tokens for a recipient
    /// @param to The recipient address
    /// @param amount The amount of tokens to mint
    function mint(address to, uint256 amount) external {
        balances[to] += amount;
    }

    /// @dev Burn tokens from the sender
    /// @param amount The amount of tokens to burn
    function burn(uint256 amount) external {
        require(balances[msg.sender] >= amount, "Insufficient balance");
        balances[msg.sender] -= amount;
    }
}
