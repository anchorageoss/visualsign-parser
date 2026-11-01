// -*- mode: Bluespec; -*-
/// NEAR token intent resolution: how the asset id carried by an intent is
/// resolved to token metadata (symbol + curated decimals) before an amount is
/// shown to a signer, and which caller-supplied metadata the resolver is
/// allowed to trust.
///
/// Modeled from (visualsign-near, presets/intents/):
///   * tokens.rs -- `resolve` (request-scoped override layer first, then the
///     compiled-in `SEEDS` table), `usable` (decimals above `MAX_DECIMALS`
///     would overflow `format_units`, so such metadata is dropped), and
///     `seeded_decimals` (the "is this asset already curated" question).
///   * render.rs -- `render_intent_body`'s per-kind dispatch,
///     `intent_consumes_token_registry` (whether an envelope carrying only
///     this intent builds the registry at all), `token_amount_field`, and
///     `render_mt_withdraw`.
///   * token_signature.rs -- `try_extract_from_chain_metadata`'s gap-fill
///     rule: an UNATTRIBUTED (unsigned) override entry for an asset that is
///     already curated makes the WHOLE extraction refuse, so a caller cannot
///     quietly restate the decimals of a curated balance.
///   * convert.rs -- the gate/dispatch parity table, which asserts that the
///     set of kinds `intent_consumes_token_registry` recognizes is exactly
///     the set whose renderer reads the registry.
///
/// Strings carry no structure in Quint, so an asset id is modeled as the
/// parsed record the code works with: `<standard>:<contract>` plus, for a
/// NEP-245 multi-token, the `mt_token_id` tail -- itself described by the
/// standard it parses as (`defuse_core::token_id::TokenId`) and, when that is
/// NEP-141, the contract it names. `OPAQUE` is a tail that is not a `TokenId`
/// at all.
module near_token_intent_resolution {

  // ----------------------------------------------------------------- domain

  pure val NO_STANDARD: str = ""
  pure val NEP141: str = "nep141"
  pure val NEP171: str = "nep171"
  pure val NEP245: str = "nep245"
  /// An `mt_token_id` that does not round-trip as a `TokenId`.
  pure val OPAQUE: str = "opaque"

  /// The four outcomes of `mt_token_id.parse::<TokenId>()`.
  pure val MT_TOKEN_ID_KINDS: Set[str] = Set(NEP141, NEP171, NEP245, OPAQUE)

  /// NEP-141 contracts. `wrap.near` is the real seeded wNEAR balance; the
  /// other stands for any contract `SEEDS` does not cover. Instrumentation
  /// maps the real account id to one of these symbols -- only its equality
  /// with a registry key or a seed key drives any branch.
  pure val WRAP_NEAR: str = "wrap.near"
  pure val UNSEEDED_TOKEN: str = "unseeded.near"
  pure val NEP141_CONTRACTS: Set[str] = Set(WRAP_NEAR, UNSEEDED_TOKEN)

  /// An NFT contract, for the `nep171:` entries a `token_diff`/`transfer` map
  /// can carry.
  pure val NFT_CONTRACT: str = "nft.near"

  /// Multi-token contracts: a defuse-style one that represents its own held
  /// balances as multi-tokens, and an independent one with its own token_id
  /// convention.
  pure val DEFUSE_CONTRACT: str = "defuse.near"
  pure val MARKET_CONTRACT: str = "market.near"
  pure val MT_CONTRACTS: Set[str] = Set(DEFUSE_CONTRACT, MARKET_CONTRACT)

  type AssetId = {
    standard: str,
    contract: str,
    innerStandard: str,
    innerContract: str,
  }

  pure val NO_ASSET: AssetId =
    { standard: NO_STANDARD, contract: "", innerStandard: NO_STANDARD, innerContract: "" }

  pure def nep141Asset(contract: str): AssetId =
    { standard: NEP141, contract: contract, innerStandard: NO_STANDARD, innerContract: "" }

  pure def nep171Asset(contract: str): AssetId =
    { standard: NEP171, contract: contract, innerStandard: NO_STANDARD, innerContract: "" }

  /// `nep245:<contract>:<mt_token_id>` -- the asset id shape a `token_diff`
  /// or `transfer` map already produces for a multi-token entry. Only a
  /// NEP-141 tail names a contract; every other tail is indistinguishable for
  /// resolution, so it normalizes away and the state space stays small.
  pure def mtAsset(contract: str, mtTokenIdKind: str, mtTokenIdContract: str): AssetId =
    { standard: NEP245,
      contract: contract,
      innerStandard: mtTokenIdKind,
      innerContract: if (mtTokenIdKind == NEP141) mtTokenIdContract else "" }

  pure val MT_ASSET_IDS: Set[AssetId] =
    tuples(MT_CONTRACTS, MT_TOKEN_ID_KINDS, NEP141_CONTRACTS)
      .map(t => mtAsset(t._1, t._2, t._3))

  /// Every asset id a caller can key an override by, or an intent can carry.
  pure val ASSET_IDS: Set[AssetId] =
    NEP141_CONTRACTS.map(c => nep141Asset(c))
      .union(Set(nep171Asset(NFT_CONTRACT)))
      .union(MT_ASSET_IDS)

  /// The eleven `Intent` kinds.
  pure val TOKEN_DIFF: str = "token_diff"
  pure val TRANSFER: str = "transfer"
  pure val FT_WITHDRAW: str = "ft_withdraw"
  pure val MT_WITHDRAW: str = "mt_withdraw"
  pure val NFT_WITHDRAW: str = "nft_withdraw"
  pure val NATIVE_WITHDRAW: str = "native_withdraw"
  pure val ADD_PUBLIC_KEY: str = "add_public_key"
  pure val REMOVE_PUBLIC_KEY: str = "remove_public_key"
  pure val INVALIDATE_NONCES: str = "invalidate_nonces"
  pure val STORAGE_DEPOSIT: str = "storage_deposit"
  pure val SET_AUTH_BY_PREDECESSOR_ID: str = "set_auth_by_predecessor_id"

  /// The kinds whose payload carries an (asset id, amount) pair other than
  /// `mt_withdraw`: `token_diff` and `transfer` carry a map (whose keys may be
  /// NEP-141, NEP-171 or NEP-245 asset ids), `ft_withdraw` a single NEP-141
  /// token.
  pure val TOKEN_BEARING_KINDS: Set[str] = Set(TOKEN_DIFF, TRANSFER, FT_WITHDRAW)

  /// The kinds that render no amount at all.
  pure val PLAIN_KINDS: Set[str] = Set(
    NFT_WITHDRAW, NATIVE_WITHDRAW, ADD_PUBLIC_KEY, REMOVE_PUBLIC_KEY,
    INVALIDATE_NONCES, STORAGE_DEPOSIT, SET_AUTH_BY_PREDECESSOR_ID,
  )

  pure val INTENT_KINDS: Set[str] =
    TOKEN_BEARING_KINDS.union(Set(MT_WITHDRAW)).union(PLAIN_KINDS)

  /// `format_units` scales by `10^decimals`, which overflows `u128` above 38,
  /// so metadata proposing more is unusable. A real trace logs
  /// `min(decimals, MAX_DECIMALS + 1)`: everything above the boundary behaves
  /// identically, so the buckets below cover the u8 the wire carries.
  pure val MAX_DECIMALS: int = 38
  pure val DECIMALS: Set[int] = Set(0, 8, 24, MAX_DECIMALS + 1)

  /// Base-unit counts, clamped: the code logs `min(units, MAX_UNITS)`. No
  /// branch reads the magnitude -- what a signer sees turns on the decimals
  /// and symbol the amount is rendered with.
  pure val MAX_UNITS: int = 2

  /// At most this many entries in one caller-supplied `token_mappings` map.
  pure val MAX_ENTRIES: int = 2

  // ------------------------------------------------------------- resolution

  pure val PROV_NONE: str = "none"
  pure val PROV_OVERRIDE: str = "override"
  pure val PROV_SEED: str = "seed"

  /// `TokenMeta`, plus the asset id the metadata was actually keyed under
  /// (`source`) and whether it was reached through another asset's entry
  /// (`borrowed`) -- what a signer's amount is attributed to.
  type TokenMeta = {
    symbol: str,
    decimals: int,
    provenance: str,
    borrowed: bool,
    source: AssetId,
  }

  pure val UNRESOLVED: TokenMeta =
    { symbol: "", decimals: 0, provenance: PROV_NONE, borrowed: false, source: NO_ASSET }

  pure def isResolved(meta: TokenMeta): bool = meta.provenance != PROV_NONE

  /// One caller-supplied `token_mappings` entry. `attributed` is the
  /// signature/allowlist verdict: a signed entry from an allowlisted curator
  /// speaks for itself, an unsigned one does not.
  type Proposal = { attributed: bool, decimals: int }

  pure val NO_OVERRIDES: AssetId -> Proposal = Map()

  pure def hasOverride(reg: AssetId -> Proposal, a: AssetId): bool =
    reg.keys().contains(a)

  pure def overrideFor(reg: AssetId -> Proposal, a: AssetId): TokenMeta =
    if (hasOverride(reg, a))
      { symbol: "OVERRIDE",
        decimals: reg.get(a).decimals,
        provenance: PROV_OVERRIDE,
        borrowed: false,
        source: a }
    else UNRESOLVED

  /// The compiled-in `SEEDS` table, reduced to the one curated NEP-141
  /// balance every branch of this component turns on.
  pure def seedFor(a: AssetId): TokenMeta =
    if (a == nep141Asset(WRAP_NEAR))
      { symbol: "wNEAR", decimals: 24, provenance: PROV_SEED, borrowed: false, source: a }
    else UNRESOLVED

  /// `usable`: metadata whose decimals would overflow `format_units` is
  /// dropped rather than used.
  pure def usableOr(meta: TokenMeta): TokenMeta =
    if (isResolved(meta) and meta.decimals <= MAX_DECIMALS) meta else UNRESOLVED

  /// `tokens::resolve`: the request-scoped override layer first, then the
  /// compiled-in seed table. An entry keyed by the asset SHADOWS the seed even
  /// when its decimals make it unusable -- the code returns `usable(meta)`
  /// there rather than falling through.
  pure def resolve(a: AssetId, reg: AssetId -> Proposal): TokenMeta =
    if (hasOverride(reg, a)) usableOr(overrideFor(reg, a))
    else usableOr(seedFor(a))

  pure val NO_DECIMALS: int = -1

  /// `tokens::seeded_decimals`: the curated `decimals` for `a`, or
  /// `NO_DECIMALS` when `SEEDS` does not cover it. This is the "already
  /// curated" question the gap-fill rule asks.
  pure def seededDecimals(a: AssetId): int =
    val seeded = seedFor(a)
    if (isResolved(seeded)) seeded.decimals else NO_DECIMALS

  // ---------------------------------------------------------------- dispatch

  /// `intent_consumes_token_registry`: whether an envelope carrying only this
  /// intent builds the token registry at all.
  pure def gateConsumesRegistry(kind: str): bool =
    Set(TOKEN_DIFF, TRANSFER, FT_WITHDRAW).contains(kind)

  /// Whether this kind's renderer actually reaches `tokens::resolve`. The gate
  /// above must agree with this, or a caller-supplied override is silently
  /// dropped (or built for nothing) -- the parity `convert.rs` asserts.
  pure def renderReadsRegistry(kind: str): bool =
    Set(TOKEN_DIFF, TRANSFER, FT_WITHDRAW).contains(kind)

  // ----------------------------------------------------------------- render

  pure val SHOWN_NONE: str = "none"
  /// An `AmountV2`: scaled by the resolved decimals and labelled with a symbol.
  pure val SHOWN_RESOLVED: str = "resolved"
  /// The honest unresolved form: raw base units and the asset id, no symbol.
  pure val SHOWN_RAW: str = "raw"

  type Rendered = {
    kind: str,
    asset: AssetId,
    units: int,
    gateBuiltRegistry: bool,
    registryBuilt: bool,
    readsRegistry: bool,
    failed: bool,
    amountShown: str,
    displaySymbol: str,
    displayDecimals: int,
    decimalsSource: AssetId,
    provenance: str,
    borrowed: bool,
    entryFields: int,
    amountFusedWithTokenId: bool,
    shadowedByUnusableOverride: bool,
  }

  pure val NO_RENDER: Rendered = {
    kind: "",
    asset: NO_ASSET,
    units: 0,
    gateBuiltRegistry: false,
    registryBuilt: false,
    readsRegistry: false,
    failed: false,
    amountShown: SHOWN_NONE,
    displaySymbol: "",
    displayDecimals: 0,
    decimalsSource: NO_ASSET,
    provenance: PROV_NONE,
    borrowed: false,
    entryFields: 0,
    amountFusedWithTokenId: false,
    shadowedByUnusableOverride: false,
  }

  /// `token_amount_field`: resolve `asset` through the effective registry,
  /// then render either a scaled, symbol-labelled `AmountV2` or the honest
  /// unresolved text.
  pure def tokenAmountField(
    kind: str,
    asset: AssetId,
    units: int,
    reg: AssetId -> Proposal,
    gateBuilt: bool,
    built: bool,
  ): Rendered =
    val meta = resolve(asset, reg)
    { ...NO_RENDER,
      kind: kind,
      asset: asset,
      units: units,
      gateBuiltRegistry: gateBuilt,
      registryBuilt: built,
      readsRegistry: true,
      amountShown: if (isResolved(meta)) SHOWN_RESOLVED else SHOWN_RAW,
      displaySymbol: meta.symbol,
      displayDecimals: if (isResolved(meta)) meta.decimals else 0,
      decimalsSource: meta.source,
      provenance: meta.provenance,
      borrowed: meta.borrowed,
      entryFields: 1,
      shadowedByUnusableOverride: hasOverride(reg, asset) and not(isResolved(meta)),
    }

  /// `render_mt_withdraw`'s per-entry rendering: each `(token_id, amount)`
  /// pair becomes ONE composite text field, `"<charset_safe(token_id)>
  /// x<amount>"` -- the raw base-unit count sharing the token-id line, with no
  /// symbol, no decimal scaling, and no registry lookup at all.
  pure def renderMtEntry(
    asset: AssetId,
    units: int,
    reg: AssetId -> Proposal,
    gateBuilt: bool,
    built: bool,
  ): Rendered =
    { ...tokenAmountField(MT_WITHDRAW, asset, units, NO_OVERRIDES, gateBuilt, built),
      readsRegistry: false,
      entryFields: 1,
      amountFusedWithTokenId: true,
    }

  // ------------------------------------------------------------------ state

  /// One `try_extract_from_chain_metadata` outcome.
  type Extraction = {
    rejected: bool,
    hadUnattributed: bool,
    byMtAliasKey: bool,
    byDirectKey: bool,
    installed: int,
  }

  /// Caller-supplied `token_mappings` entries staged for extraction.
  var proposed: AssetId -> Proposal
  /// The installed request-scoped override layer.
  var registry: AssetId -> Proposal
  /// Sticky accumulator of every extraction outcome.
  var extractions: Set[Extraction]
  /// Sticky accumulator of every intent rendering. Write-only: no transition
  /// reads it, it exists so behavior observations hold across a whole run.
  var seen: Set[Rendered]
  /// The most recent rendering.
  var lastRender: Rendered

  action specInit: bool = all {
    proposed' = NO_OVERRIDES,
    registry' = NO_OVERRIDES,
    extractions' = Set(),
    seen' = Set(),
    lastRender' = NO_RENDER,
  }

  // ---------------------------------------------------------------- actions

  /// One entry of a caller-supplied `token_mappings` map arrives.
  action proposeMetadataEntry(assetId: AssetId, attributed: bool, decimals: int): bool = all {
    ASSET_IDS.contains(assetId),
    DECIMALS.contains(decimals),
    proposed.keys().size() < MAX_ENTRIES,
    not(proposed.keys().contains(assetId)),
    proposed' = proposed.put(assetId, { attributed: attributed, decimals: decimals }),
    registry' = registry,
    extractions' = extractions,
    seen' = seen,
    lastRender' = lastRender,
  }

  /// `try_extract_from_chain_metadata`: the gap-fill rule. An unattributed
  /// entry for an asset that is already curated makes the WHOLE extraction
  /// refuse, so nothing of that bundle is installed.
  action extractRegistry: bool = {
    val blockers = proposed.keys().filter(a =>
      not(proposed.get(a).attributed) and seededDecimals(a) != NO_DECIMALS)
    val rejected = blockers.size() > 0
    all {
      proposed.keys().size() > 0,
      registry' = if (rejected) NO_OVERRIDES else proposed,
      proposed' = NO_OVERRIDES,
      extractions' = extractions.union(Set({
        rejected: rejected,
        hadUnattributed: proposed.keys().exists(a => not(proposed.get(a).attributed)),
        byMtAliasKey: blockers.exists(a => a.standard == NEP245),
        byDirectKey: blockers.exists(a => a.standard != NEP245),
        installed: if (rejected) 0 else proposed.keys().size(),
      })),
      seen' = seen,
      lastRender' = lastRender,
    }
  }

  /// `render_intent_body` for a kind carrying an (asset id, amount) pair other
  /// than `mt_withdraw`. `siblingConsumesRegistry` is whether some OTHER
  /// intent in the same envelope already made the gate build the registry.
  action renderTokenIntent(
    kind: str,
    assetId: AssetId,
    units: int,
    siblingConsumesRegistry: bool,
  ): bool = {
    val gateBuilt = gateConsumesRegistry(kind)
    val built = gateBuilt or siblingConsumesRegistry
    val effective = if (built) registry else NO_OVERRIDES
    val rendered = tokenAmountField(kind, assetId, units, effective, gateBuilt, built)
    all {
      TOKEN_BEARING_KINDS.contains(kind),
      ASSET_IDS.contains(assetId),
      units >= 0,
      units <= MAX_UNITS,
      // `ft_withdraw` names a single NEP-141 token, never a map key.
      (kind == FT_WITHDRAW) implies (assetId.standard == NEP141),
      lastRender' = rendered,
      seen' = seen.union(Set(rendered)),
      proposed' = proposed,
      registry' = registry,
      extractions' = extractions,
    }
  }

  /// `render_mt_withdraw`: the asset id is `nep245:<w.token>:<token_id>`.
  /// `countsMatch` is `token_ids.len() == amounts.len()`; a mismatch is a
  /// `ValidationError` and the intent renders nothing.
  action renderMtWithdraw(
    mtContract: str,
    mtTokenIdKind: str,
    mtTokenIdContract: str,
    units: int,
    countsMatch: bool,
    siblingConsumesRegistry: bool,
  ): bool = {
    val assetId = mtAsset(mtContract, mtTokenIdKind, mtTokenIdContract)
    val gateBuilt = gateConsumesRegistry(MT_WITHDRAW)
    val built = gateBuilt or siblingConsumesRegistry
    val effective = if (built) registry else NO_OVERRIDES
    val rendered =
      if (countsMatch) renderMtEntry(assetId, units, effective, gateBuilt, built)
      else { ...NO_RENDER,
             kind: MT_WITHDRAW,
             asset: assetId,
             units: units,
             gateBuiltRegistry: gateBuilt,
             registryBuilt: built,
             failed: true }
    all {
      MT_CONTRACTS.contains(mtContract),
      MT_TOKEN_ID_KINDS.contains(mtTokenIdKind),
      NEP141_CONTRACTS.contains(mtTokenIdContract),
      units >= 0,
      units <= MAX_UNITS,
      lastRender' = rendered,
      seen' = seen.union(Set(rendered)),
      proposed' = proposed,
      registry' = registry,
      extractions' = extractions,
    }
  }

  /// `render_intent_body` for a kind that renders no amount.
  action renderPlainIntent(kind: str): bool = {
    val rendered = { ...NO_RENDER,
                     kind: kind,
                     gateBuiltRegistry: gateConsumesRegistry(kind),
                     registryBuilt: gateConsumesRegistry(kind),
                     readsRegistry: renderReadsRegistry(kind) }
    all {
      PLAIN_KINDS.contains(kind),
      lastRender' = rendered,
      seen' = seen.union(Set(rendered)),
      proposed' = proposed,
      registry' = registry,
      extractions' = extractions,
    }
  }

  // --------------------------------------------------- behavior observations

  /// An `mt_withdraw` showed the signer a scaled, symbol-labelled amount.
  val MtWithdrawAmountResolved: bool =
    seen.exists(r => r.kind == MT_WITHDRAW and r.amountShown == SHOWN_RESOLVED)

  /// An `mt_withdraw` showed the signer raw base units with no symbol.
  val MtWithdrawAmountRaw: bool =
    seen.exists(r => r.kind == MT_WITHDRAW and r.amountShown == SHOWN_RAW)

  /// An amount was rendered with metadata borrowed from a DIFFERENT asset id
  /// than the one the intent named.
  val AmountResolvedByBorrowedMetadata: bool =
    seen.exists(r => r.borrowed)

  /// The registry gate itself built the token registry for an `mt_withdraw`,
  /// so a standalone `mt_withdraw` envelope has an override layer at all.
  val MtWithdrawGateBuiltRegistry: bool =
    seen.exists(r => r.kind == MT_WITHDRAW and r.gateBuiltRegistry)

  /// A multi-token asset resolved through an override keyed by that exact
  /// `nep245:...` id, rather than through anything borrowed.
  val MtAssetResolvedByExactOverride: bool =
    seen.exists(r => and {
      r.asset.standard == NEP245,
      r.provenance == PROV_OVERRIDE,
      not(r.borrowed),
    })

  /// A multi-token entry whose `mt_token_id` is not a NEP-141 balance was left
  /// unresolved rather than misattributed.
  val NonFungibleMtTokenLeftUnresolved: bool =
    seen.exists(r => and {
      r.asset.standard == NEP245,
      r.asset.innerStandard == NEP171,
      r.amountShown == SHOWN_RAW,
    })

  /// The gap-fill rule refused a bundle because an unattributed entry was
  /// keyed by a multi-token ALIAS of an already-curated balance.
  val UnattributedMtAliasEntryRejected: bool =
    extractions.exists(x => x.rejected and x.byMtAliasKey)

  /// The gap-fill rule refused a bundle because an unattributed entry was
  /// keyed by an already-curated balance directly.
  val UnattributedDirectKeyEntryRejected: bool =
    extractions.exists(x => x.rejected and x.byDirectKey)

  /// An unattributed entry was installed: nothing curated covers that asset,
  /// so the caller is allowed to fill the gap.
  val UnattributedEntryInstalled: bool =
    extractions.exists(x => not(x.rejected) and x.hadUnattributed)

  /// An `mt_withdraw` was refused for a token_ids/amounts length mismatch.
  val MtWithdrawRejectedForLengthMismatch: bool =
    seen.exists(r => r.kind == MT_WITHDRAW and r.failed)

  /// An override entry with unusable decimals shadowed the curated seed, so
  /// the amount fell back to raw base units.
  val UnusableOverrideShadowedCuratedSeed: bool =
    seen.exists(r => r.shadowedByUnusableOverride)

  /// The token-id text field carried the amount on its own line.
  val MtTokenFieldCarriedTheAmount: bool =
    seen.exists(r => r.amountFusedWithTokenId)

  // ------------------------------------------------------------- invariants

  /// The gate and the renderers agree on which kinds consult the registry, so
  /// a caller-supplied override is never built for nothing nor silently
  /// dropped (`convert.rs`'s parity table).
  val gateMatchesRenderRegistryUse: bool =
    INTENT_KINDS.forall(k => gateConsumesRegistry(k) == renderReadsRegistry(k))

  /// No renderer ever reads a registry its own envelope did not build.
  val noRenderReadsAnUnbuiltRegistry: bool =
    seen.forall(r => r.readsRegistry implies r.registryBuilt)

  /// Decimals borrowed from another asset only ever come from the NEP-141
  /// balance the multi-token id itself names -- never from the multi-token
  /// contract's own id, an NFT, or a nested multi-token.
  val borrowedDecimalsNameTheirOwnBalance: bool =
    seen.forall(r => or {
      r.decimalsSource == r.asset,
      r.decimalsSource == NO_ASSET,
      and {
        r.asset.standard == NEP245,
        r.asset.innerStandard == NEP141,
        r.decimalsSource == nep141Asset(r.asset.innerContract),
      },
    })

  /// Any asset whose amount can be scaled from curated seed metadata is also
  /// covered by the gap-fill rule -- resolution and gap-fill protection never
  /// drift apart, so no asset can be resolvable yet exempt.
  val curatedAssetsStayGapFillProtected: bool =
    ASSET_IDS.forall(a =>
      isResolved(resolve(a, NO_OVERRIDES)) implies (seededDecimals(a) != NO_DECIMALS))

  /// No installed unattributed override can restate the metadata of an asset
  /// whose amount would otherwise be scaled from curated seed metadata.
  val unattributedOverridesNeverDisplaceCuratedMetadata: bool =
    registry.keys().forall(a =>
      registry.get(a).attributed or not(isResolved(resolve(a, NO_OVERRIDES))))

  /// `a` names a balance the compiled-in table curates: either by the
  /// balance's own asset id, or as the `nep245:<contract>:<token_id>` alias a
  /// defuse-style contract uses for one of its own held NEP-141 balances.
  pure def namesCuratedBalance(a: AssetId): bool =
    or {
      seededDecimals(a) != NO_DECIMALS,
      and {
        a.standard == NEP245,
        a.innerStandard == NEP141,
        seededDecimals(nep141Asset(a.innerContract)) != NO_DECIMALS,
      },
    }

  /// The installed override layer never carries an unauthenticated entry for
  /// an asset id that names a balance this parser already curates.
  val noUnattributedEntryForACuratedBalance: bool =
    registry.keys().forall(a =>
      registry.get(a).attributed or not(namesCuratedBalance(a)))

  /// Two renderings of the SAME asset id in one request show the signer the
  /// same thing: which intent kind carries a balance never decides whether it
  /// is named and scaled or shown as raw base units.
  val sameAssetShownTheSameWayAcrossIntentKinds: bool =
    seen.forall(r => seen.forall(q =>
      (r.asset == q.asset and not(r.failed) and not(q.failed)) implies (and {
        r.amountShown == q.amountShown,
        r.displayDecimals == q.displayDecimals,
      })))

  /// An unresolved amount is honest about it: raw base units, no symbol, no
  /// borrowed decimals.
  val unresolvedAmountsNeverCarryASymbol: bool =
    seen.forall(r => (r.amountShown == SHOWN_RAW) implies (and {
      r.displaySymbol == "",
      r.displayDecimals == 0,
      r.decimalsSource == NO_ASSET,
    }))

  /// Every intent that carries an amount shows one, unless it was refused
  /// outright.
  val everyAmountBearingIntentShowsAnAmount: bool =
    seen.forall(r =>
      (TOKEN_BEARING_KINDS.union(Set(MT_WITHDRAW)).contains(r.kind))
        implies (r.failed or r.amountShown != SHOWN_NONE))
}

/// Oracle main: replay entry point. Every nondet pick is branch-local and
/// named exactly as the argument the instrumented code logs.
module NearTokenIntentResolutionOracle {
  import near_token_intent_resolution.*

  action init: bool = specInit

  action step: bool = any {
    {
      nondet assetId = ASSET_IDS.oneOf()
      nondet attributed = Bool.oneOf()
      nondet decimals = DECIMALS.oneOf()
      proposeMetadataEntry(assetId, attributed, decimals)
    },
    extractRegistry,
    {
      nondet kind = TOKEN_BEARING_KINDS.oneOf()
      nondet assetId = ASSET_IDS.oneOf()
      nondet units = 0.to(MAX_UNITS).oneOf()
      nondet siblingConsumesRegistry = Bool.oneOf()
      renderTokenIntent(kind, assetId, units, siblingConsumesRegistry)
    },
    {
      nondet mtContract = MT_CONTRACTS.oneOf()
      nondet mtTokenIdKind = MT_TOKEN_ID_KINDS.oneOf()
      nondet mtTokenIdContract = NEP141_CONTRACTS.oneOf()
      nondet units = 0.to(MAX_UNITS).oneOf()
      nondet countsMatch = Bool.oneOf()
      nondet siblingConsumesRegistry = Bool.oneOf()
      renderMtWithdraw(
        mtContract, mtTokenIdKind, mtTokenIdContract, units,
        countsMatch, siblingConsumesRegistry,
      )
    },
    {
      nondet kind = PLAIN_KINDS.oneOf()
      renderPlainIntent(kind)
    },
  }
}

module NearTokenIntentResolutionTest {
  import near_token_intent_resolution.*

  action init: bool = specInit

  /// Happy path: an `ft_withdraw` of a seeded balance is scaled and labelled.
  run ftWithdrawResolvesSeededBalanceTest =
    specInit
      .then(renderTokenIntent(FT_WITHDRAW, nep141Asset(WRAP_NEAR), 2, false))
      .expect(and {
        lastRender.amountShown == SHOWN_RESOLVED,
        lastRender.displaySymbol == "wNEAR",
        lastRender.displayDecimals == 24,
        lastRender.decimalsSource == nep141Asset(WRAP_NEAR),
        not(lastRender.borrowed),
      })

  /// An `mt_withdraw` whose `token_id` names a NEP-141 balance a defuse-style
  /// contract holds: the gate does not build the registry for a standalone
  /// `mt_withdraw` envelope, and the renderer would not read it anyway -- the
  /// amount shares the token-id line as raw base units.
  run mtWithdrawWrappingASeededBalanceTest =
    specInit
      .then(renderMtWithdraw(DEFUSE_CONTRACT, NEP141, WRAP_NEAR, 2, true, false))
      .expect(and {
        not(lastRender.gateBuiltRegistry),
        not(lastRender.readsRegistry),
        lastRender.amountShown == SHOWN_RAW,
        lastRender.displaySymbol == "",
        lastRender.amountFusedWithTokenId,
        lastRender.entryFields == 1,
        MtWithdrawAmountRaw,
        MtTokenFieldCarriedTheAmount,
      })

  /// The same withdraw beside an intent that DOES build the registry: the
  /// registry exists, but this renderer never consults it.
  run mtWithdrawBesideARegistryBuildingSiblingTest =
    specInit
      .then(renderMtWithdraw(DEFUSE_CONTRACT, NEP141, WRAP_NEAR, 2, true, true))
      .expect(and {
        lastRender.registryBuilt,
        not(lastRender.gateBuiltRegistry),
        lastRender.amountShown == SHOWN_RAW,
      })

  /// An independent multi-token contract's own token_id convention does not
  /// name any asset, so the amount stays honestly unresolved.
  run mtWithdrawOpaqueTokenIdStaysUnresolvedTest =
    specInit
      .then(renderMtWithdraw(MARKET_CONTRACT, OPAQUE, WRAP_NEAR, 2, true, false))
      .expect(and {
        lastRender.amountShown == SHOWN_RAW,
        lastRender.provenance == PROV_NONE,
        lastRender.displaySymbol == "",
        MtWithdrawAmountRaw,
      })

  /// A `token_id` that parses, but as an NFT rather than a fungible balance,
  /// has no decimals to borrow.
  run mtWithdrawWrappingAnNftStaysUnresolvedTest =
    specInit
      .then(renderMtWithdraw(DEFUSE_CONTRACT, NEP171, WRAP_NEAR, 2, true, false))
      .expect(and {
        lastRender.amountShown == SHOWN_RAW,
        NonFungibleMtTokenLeftUnresolved,
      })

  /// A token_ids/amounts length mismatch refuses the whole intent.
  run mtWithdrawLengthMismatchTest =
    specInit
      .then(renderMtWithdraw(DEFUSE_CONTRACT, NEP141, WRAP_NEAR, 2, false, false))
      .expect(and {
        lastRender.failed,
        lastRender.amountShown == SHOWN_NONE,
        MtWithdrawRejectedForLengthMismatch,
      })

  /// An unattributed entry keyed by a multi-token ALIAS of the seeded
  /// `wrap.near` balance: the gap-fill rule does not recognize the alias, so
  /// the bundle is installed even though the direct key is protected.
  run unattributedMtAliasEntryTest =
    specInit
      .then(proposeMetadataEntry(mtAsset(DEFUSE_CONTRACT, NEP141, WRAP_NEAR), false, 8))
      .then(extractRegistry)
      .expect(and {
        registry.keys().size() == 1,
        UnattributedEntryInstalled,
        not(UnattributedMtAliasEntryRejected),
      })

  /// The same entry keyed by the curated balance DIRECTLY is refused, and the
  /// refusal takes the whole bundle with it.
  run unattributedDirectKeyEntryRejectedTest =
    specInit
      .then(proposeMetadataEntry(nep141Asset(WRAP_NEAR), false, 8))
      .then(proposeMetadataEntry(nep141Asset(UNSEEDED_TOKEN), false, 8))
      .then(extractRegistry)
      .expect(and {
        registry.keys().size() == 0,
        UnattributedDirectKeyEntryRejected,
      })

  /// An override keyed by the exact multi-token asset id wins over anything
  /// else -- "specific beats borrowed".
  run mtSpecificOverrideBeatsTheUnderlyingBalanceTest =
    specInit
      .then(proposeMetadataEntry(mtAsset(DEFUSE_CONTRACT, NEP141, WRAP_NEAR), true, 8))
      .then(extractRegistry)
      .then(renderTokenIntent(TRANSFER, mtAsset(DEFUSE_CONTRACT, NEP141, WRAP_NEAR), 2, false))
      .expect(and {
        lastRender.amountShown == SHOWN_RESOLVED,
        lastRender.displayDecimals == 8,
        lastRender.provenance == PROV_OVERRIDE,
        not(lastRender.borrowed),
        MtAssetResolvedByExactOverride,
      })

  /// An attributed override whose decimals would overflow `format_units` is
  /// dropped, and it shadows the curated seed rather than falling through.
  run unusableOverrideShadowsCuratedSeedTest =
    specInit
      .then(proposeMetadataEntry(nep141Asset(WRAP_NEAR), true, MAX_DECIMALS + 1))
      .then(extractRegistry)
      .then(renderTokenIntent(FT_WITHDRAW, nep141Asset(WRAP_NEAR), 2, false))
      .expect(and {
        lastRender.amountShown == SHOWN_RAW,
        lastRender.shadowedByUnusableOverride,
        UnusableOverrideShadowedCuratedSeed,
      })

  /// The renderer is a stream: several intents in a row, each rendered from
  /// the built-up registry, with sticky observations across all of them.
  run threeIntentStreamTest =
    specInit
      .then(renderTokenIntent(FT_WITHDRAW, nep141Asset(WRAP_NEAR), 1, false))
      .then(renderMtWithdraw(DEFUSE_CONTRACT, OPAQUE, WRAP_NEAR, 1, true, false))
      .then(renderTokenIntent(TRANSFER, nep171Asset(NFT_CONTRACT), 1, false))
      .expect(and {
        seen.size() == 3,
        lastRender.kind == TRANSFER,
        lastRender.amountShown == SHOWN_RAW,
        MtWithdrawAmountRaw,
      })
}
