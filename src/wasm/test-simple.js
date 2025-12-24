// Simple Node.js test for VisualSign WASM
const { parse_ethereum_transaction, parse_transaction } = require('./pkg-node/visualsign_wasm.js');

// Example Ethereum transactions
const testTransactions = {
  ethTransfer: '0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83',
  erc20Transfer: '0xf8a9808504a817c80082520894dac17f958d2ee523a2206206994597c13d831ec780b844a9059cbb0000000000000000000000001234567890123456789012345678901234567890000000000000000000000000000000000000000000000000000000003b9aca0025a0abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890a01234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef',
};

console.log('🚀 VisualSign WASM Test Suite\n');
console.log('='.repeat(60));

// Test 1: Parse ETH transfer
console.log('\n📝 Test 1: Parsing simple ETH transfer');
try {
  const result1 = parse_ethereum_transaction(testTransactions.ethTransfer);
  const parsed1 = JSON.parse(result1);
  console.log('✅ Success!');
  console.log('   Type:', parsed1.PayloadType);
  console.log('   Title:', parsed1.Title);
  console.log('   Fields:', parsed1.Fields.length);

  // Show a few fields
  console.log('\n   Sample fields:');
  parsed1.Fields.slice(0, 3).forEach(field => {
    console.log(`   - ${field.Label}: ${field.FallbackText}`);
  });
} catch (error) {
  console.error('❌ Error:', error.message);
}

console.log('\n' + '='.repeat(60));

// Test 2: Parse ERC-20 transfer
console.log('\n📝 Test 2: Parsing ERC-20 transfer');
try {
  const result2 = parse_ethereum_transaction(testTransactions.erc20Transfer);
  const parsed2 = JSON.parse(result2);
  console.log('✅ Success!');
  console.log('   Type:', parsed2.PayloadType);
  console.log('   Title:', parsed2.Title);
  console.log('   Fields:', parsed2.Fields.length);
} catch (error) {
  console.error('❌ Error:', error.message);
}

console.log('\n' + '='.repeat(60));

// Test 3: Auto-detect
console.log('\n📝 Test 3: Auto-detect with parse_transaction()');
try {
  const result3 = parse_transaction(testTransactions.ethTransfer);
  const parsed3 = JSON.parse(result3);
  console.log('✅ Success!');
  console.log('   Auto-detected as:', parsed3.PayloadType);
} catch (error) {
  console.error('❌ Error:', error.message);
}

console.log('\n' + '='.repeat(60));

// Test 4: Error handling
console.log('\n📝 Test 4: Error handling (invalid transaction)');
try {
  parse_ethereum_transaction('0xinvalid');
  console.log('❌ FAILED - should have thrown an error!');
} catch (error) {
  console.log('✅ Success! Correctly caught error');
  console.log('   Error:', error.message);
}

console.log('\n' + '='.repeat(60));
console.log('\n🎉 All tests completed!\n');
