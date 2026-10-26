#!/usr/bin/env python3
"""Independent SDK vectors. Run with eth-account/msgpack installed and SDK checkout on PYTHONPATH.
SDK must be checked out at 2fdb18f9517675ea03695a0962bd19eece9c83f0.
Keys 1, 2 and 3 are public disposable test vectors; never fund these accounts.
"""
import json
from pathlib import Path
from eth_account import Account
from hyperliquid.utils import signing as sdk

class Wallet:
    def __init__(self,n): self.wallet=Account.from_key(n.to_bytes(32,'big')); self.address=self.wallet.address
    def sign_message(self,m):
        s=self.wallet.sign_message(m);self.digest='0x'+s.message_hash.hex();return s
w1,w2,w3=Wallet(1),Wallet(2),Wallet(3)
users=sorted([w1.address.lower(),w2.address.lower()])
config={'address':'0x0000000000000000000000000000000000000005','authorizedUsers':users,'threshold':2}
nonce=1700000000000
mainnet=False
vault=None
expiry=None
A='0x0000000000000000000000000000000000000011'
T='USDC:0x6d1e7cde53ba9467b783cb7c530ce054'
order={'a':4,'b':True,'p':'1100','s':'0.2','r':False,'t':{'limit':{'tif':'Gtc'}}}
schemas={
 'usdSend':(sdk.USD_SEND_SIGN_TYPES,'HyperliquidTransaction:UsdSend'),
 'spotSend':(sdk.SPOT_TRANSFER_SIGN_TYPES,'HyperliquidTransaction:SpotSend'),
 'withdraw3':(sdk.WITHDRAW_SIGN_TYPES,'HyperliquidTransaction:Withdraw'),
 'usdClassTransfer':(sdk.USD_CLASS_TRANSFER_SIGN_TYPES,'HyperliquidTransaction:UsdClassTransfer'),
 'sendAsset':(sdk.SEND_ASSET_SIGN_TYPES,'HyperliquidTransaction:SendAsset'),
 'tokenDelegate':(sdk.TOKEN_DELEGATE_TYPES,'HyperliquidTransaction:TokenDelegate'),
 'convertToMultiSigUser':(sdk.CONVERT_TO_MULTI_SIG_USER_SIGN_TYPES,'HyperliquidTransaction:ConvertToMultiSigUser'),
 'approveAgent':([{'name':'hyperliquidChain','type':'string'},{'name':'agentAddress','type':'address'},{'name':'agentName','type':'string'},{'name':'nonce','type':'uint64'}],'HyperliquidTransaction:ApproveAgent'),
 'approveBuilderFee':([{'name':'hyperliquidChain','type':'string'},{'name':'maxFeeRate','type':'string'},{'name':'builder','type':'address'},{'name':'nonce','type':'uint64'}],'HyperliquidTransaction:ApproveBuilderFee'),
}
actions=[
 {'type':'order','orders':[order],'grouping':'na'},
 {'type':'batchModify','modifies':[{'oid':42,'order':order}]},
 {'type':'cancel','cancels':[{'a':4,'o':42}]},
 {'type':'cancelByCloid','cancels':[{'asset':4,'cloid':'0x00000000000000000000000000000001'}]},
 {'type':'scheduleCancel','time':nonce+10000},
 {'type':'updateLeverage','asset':4,'isCross':False,'leverage':3},
 {'type':'updateIsolatedMargin','asset':4,'isBuy':True,'ntli':-123456},
 {'destination':A,'amount':'12.5','time':nonce,'type':'usdSend'},
 {'destination':A,'amount':'12.5','token':T,'time':nonce,'type':'spotSend'},
 {'type':'sendAsset','destination':A,'sourceDex':'','destinationDex':'spot','token':'USDC','amount':'12.5','fromSubAccount':'','nonce':nonce},
 {'type':'usdClassTransfer','amount':'12.5','toPerp':True,'nonce':nonce},
 {'destination':A,'amount':'12.5','time':nonce,'type':'withdraw3'},
 {'validator':A,'wei':100000000,'isUndelegate':False,'nonce':nonce,'type':'tokenDelegate'},
 {'type':'approveAgent','agentAddress':A,'agentName':'disposable test agent','nonce':nonce},
 {'maxFeeRate':'0.01%','builder':A,'nonce':nonce,'type':'approveBuilderFee'},
 {'type':'convertToMultiSigUser','signers':json.dumps({'authorizedUsers':users,'threshold':1}),'nonce':nonce},
 {'type':'convertToMultiSigUser','signers':'null','nonce':nonce},
]
def request(action,n=nonce,role='member',signer=None,config=config,leader=w1.address.lower(),signatures=None):
 return {'kind':'multisig','network':'mainnet' if mainnet else 'testnet','role':role,'signer':signer or w1.address.lower(),'multisig':config,'leader':leader,'action':action,'nonce':n,'vaultAddress':vault,'expiresAfter':expiry,'signatures':signatures or []}
def sign_member(wallet,action,n):
 kind=action['type']
 if kind in schemas:
  types,primary=schemas[kind]
  return sdk.sign_multi_sig_user_signed_action_payload(wallet,action,mainnet,types,primary,config['address'],w1.address.lower())
 return sdk.sign_multi_sig_l1_action_payload(wallet,action,mainnet,vault,n,expiry,config['address'],w1.address.lower())
def emit(action,n=nonce,name=None):
 kind=action['type'];action=dict(action)
 if kind in schemas: action.update(signatureChainId='0x66eee',hyperliquidChain='Mainnet' if mainnet else 'Testnet')
 s1=sign_member(w1,action,n);digest=w1.digest;s2=sign_member(w2,action,n)
 req=request(action,n)
 env={'type':'multiSig','signatureChainId':'0x66eee','signatures':[s1,s2],'payload':{'multiSigUser':config['address'],'outerSigner':w1.address.lower(),'action':action}}
 outer=sdk.sign_multi_sig_action(w1,env,mainnet,vault,n,expiry)
 return {'name':name or kind,'request':req,'digest':digest,'signature':s1,'leader_request':request(action,n,role='leader',signatures=[s1,s2]),'leader_digest':w1.digest,'leader_signature':outer,'exchange_body':{'action':env,'nonce':n,'signature':outer,'vaultAddress':None,'expiresAfter':None}}
vectors=[emit(a,name=('removeMultisig' if a.get('signers')=='null' else a['type'])) for a in actions]
for n in range(nonce+1,nonce+10000):
 a={'type':'order','orders':[order],'grouping':'na'}
 s=sign_member(w1,a,n)
 if len(s['r'])<66 or len(s['s'])<66:
  vectors.append(emit(a,n,'leadingZeroSignature'));break
else: raise RuntimeError('no leading-zero fixture')
a={'type':'convertToMultiSigUser','signers':json.dumps({'authorizedUsers':users,'threshold':2}),'nonce':nonce}
s=sdk.sign_convert_to_multi_sig_user_action(w3,a,mainnet)
vectors.append({'name':'initialConversion','request':request(a,role='conversion',signer=w3.address.lower(),config=None,leader=None),'digest':w3.digest,'signature':s})
# Mainnet domain binding and optional vault/expiry bytes are SDK-derived too.
mainnet=True
vectors.extend([emit(actions[0], name='mainnetOrder'), emit(actions[7], name='mainnetUsdSend')])
mainnet=False
vault=A
expiry=nonce+60000
vectors.append(emit(actions[0], name='vaultAndExpiryOrder'))
flat=[]
for vector in vectors:
    for key, digest_key, signature_key in [('request','digest','signature'), ('leader_request','leader_digest','leader_signature')]:
        if key not in vector:
            continue
        request=vector[key]
        request['signingDigest']=vector[digest_key]
        flat.append({'name':vector['name']+'_'+request['role'],
                     'unsignedPayload':json.dumps(request,separators=(',', ':')),
                     'digest':vector[digest_key], 'signature':vector[signature_key],
                     'leaderActionJson':json.dumps(vector['exchange_body']['action'],separators=(',', ':')) if key=='leader_request' else None})
Path(__file__).with_name('vectors.json').write_text(json.dumps({
    'sdkRevision':'2fdb18f9517675ea03695a0962bd19eece9c83f0',
    'provenance':'Official hyperliquid-python-sdk signing functions; public test keys 1, 2, 3; no network calls.',
    'vectors':flat},indent=2)+'\n')
print('wrote',len(flat),'vectors')
