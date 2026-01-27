# Mintlify documentation guidance

## Working relationship
- You can push back on ideas—this can lead to better documentation. Cite sources and explain your reasoning when you do so
- ALWAYS ask for clarification rather than making assumptions

## Project context
- Format: MDX files with YAML frontmatter
- Config: docs.json for navigation, theme, settings
- Components: Mintlify components

## Content strategy
- Prioritize accuracy—never guess or make up information
- Document just enough for user success (see Voice Guide for tone)
- Make content evergreen when possible
- Search for existing information before adding new content. Avoid duplication unless it is done for a strategic reason
- Check existing patterns for consistency
- Start by making the smallest reasonable changes

## Frontmatter requirements for pages
- title: Clear, descriptive page title
- description: Concise summary for SEO/navigation

## Writing standards
- Second-person voice ("you")
- Prerequisites at start of procedural content
- Test all code examples before publishing
- Match style and formatting of existing pages
- Include both basic and advanced use cases
- Language tags on all code blocks
- Alt text on all images
- Relative paths for internal links

## Diagrams
- Use Mermaid for diagrams (native Mintlify support with interactive zoom/pan)
- Prefer Mermaid over ASCII art for better rendering and accessibility
- Always visually validate diagrams after creating or modifying them

### Visual validation process

Prerequisites (one-time setup):
```bash
# Install Chrome for Puppeteer
npx puppeteer browsers install chrome-headless-shell

# Install Puppeteer in /tmp (the script requires it there)
cd /tmp && npm install puppeteer
```

The Mintlify dev server must be running on port 3000. Use the screenshot script to capture pages:

```bash
# Usage: ./scripts/screenshot.js <page-path> [output-file] [scroll-offset]
./scripts/screenshot.js /getting-started
./scripts/screenshot.js /security-model#trust-boundaries shot.png 400
```

Screenshots are saved to `screenshots/` (auto-created, gitignored). Then use the Read tool to view the screenshot file.

Note: The `mmdc` CLI renders differently than Mintlify's Mermaid renderer. Always validate against the actual Mintlify site for accurate results.

## Git workflow
- NEVER use --no-verify when committing
- Ask how to handle uncommitted changes before starting
- Create a new branch when no clear branch exists for changes
- Commit frequently throughout development
- NEVER skip or disable pre-commit hooks

## Do not
- Skip frontmatter on any MDX file
- Use absolute URLs for internal links
- Include untested code examples

# Voice Guide

Voice and style guidelines for Anchorage Digital documentation.

## Core Principles

- Be concise and helpful
- Tell the truth
- Edit for clarity—writing should sound natural when read aloud
- Avoid cliches

## Voice

Anchorage Digital's voice is a **helpful, polished expert** that anticipates reader needs.

- **Natural** — Plain, direct communication. Avoid overly technical or florid language.
- **Smart** — Claims are truthful, exact, specific, and defensible.
- **Helpful** — We make things click. Always good-natured, never mean-spirited.
- **Warm** — Generous answers to implied questions. Meet uncertainty with assured, comforting guidance.

## Tone

- Positive and affirmative, but not peppy
- Use exclamation marks sparingly
- Humor is rare; when used, it's dry and purposeful

## Terminology

| Use | Don't use |
|-----|-----------|
| Anchorage Digital | Anchorage |
| Anchorage Digital trading | Anchorage Digital Trading |
| Asset support | listing |
| AUC (assets under custody) | AUM |
| Clients | Customers |
| Collect (staking rewards) | earn |
| Crypto, Digital assets | — |
| Federally chartered | federally-chartered |
| Holder | investor |
| Institutions | Institutional investors |
| Interest | yield |
| Most favorable pricing | best price execution |
| Open-source | open source |
| Platform | suite |
| qualified custodian | Qualified Custodian |
| Real-time | real time |
| Re-delegation | compounding |
| Rewards | dividends |
| Stablecoin | stable coin |
| Trade-off | trade off, tradeoff |
| Trade | invest |
| Trading limits | credit limits |
| U.S. | US |
| Allowlisted | whitelisted |

## Style Rules

**Capitalization** — Use sentence case, including for services: "Anchorage Digital trading", "How to use Anchorage Digital guide"

**Acronyms** — Write out on first use: `Central Bank Digital Currencies (CBDCs)`. Only abbreviate if used three or more times.

**Numbers** — Words for numbers below 10, numerals for 10+: "eight", "256"

**Pronouns** — Use gender-neutral pronouns (they/them) unless referring to a specific person.

- Yes: "**They** should use Anchorage Digital custody because it will help them meet **their** goals."
- No: "If the CISO wanted to check security, **he** could put **his** questions in an RFP."

**Oxford comma** — Always use it: "custody, trading, and staking"

- Yes: "I would like to thank my parents, Nathan, and Diogo."
- No: "I would like to thank my parents, Nathan and Diogo."

**Sentence spacing** — Single space between sentences.

**Em dash** — Use `—` not `-` or `---`

**Dates** — Avoid saying "yesterday"; use the day of the week instead. Use date-month-year format to avoid regional confusion: "Monday, 5 July, 2021"

**Titles** — Capitalize titles preceding names; lowercase titles that stand alone or are offset by commas.

- Yes: "Anchorage Digital President and Co-Founder Diogo Mónica"
- Yes: "The CEO is Nathan."
- No: "The General Counsel at Anchorage Digital is Georgia."

**Bulleted lists** — Use end punctuation if the preceding clause is a complete sentence. Omit punctuation for sentence fragments.

**Currencies** — Default to USD. Use $23M or $43B for shorthand.
