out/parser_app/index.json: \
	$(shell git ls-files images/parser_app src)
	$(call build,parser_app)

out/parser_cli/index.json: \
	$(shell git ls-files images/parser_cli src)
	$(call build,parser_cli)

out/parser_gateway/index.json: \
	$(shell git ls-files images/parser_gateway src)
	$(call build,parser_gateway)

out/parser_grpc_server/index.json: \
	$(shell git ls-files images/parser_grpc_server src)
	$(call build,parser_grpc_server)

out/mock_facilitator/index.json: \
	$(shell git ls-files images/mock_facilitator src)
	$(call build,mock_facilitator)

.PHONY: non-oci-docker-images
non-oci-docker-images:
	docker buildx build --load --tag anchorageoss-visualsign-parser/parser_app -f images/parser_app/Containerfile .
	docker buildx build --load --tag anchorageoss-visualsign-parser/parser_gateway -f images/parser_gateway/Containerfile .
	docker buildx build --load --tag anchorageoss-visualsign-parser/parser_grpc_server -f images/parser_grpc_server/Containerfile .
	docker buildx build --load --tag anchorageoss-visualsign-parser/mock_facilitator -f images/mock_facilitator/Containerfile .

# ── Local dev stacks ────────────────────────────────────────────────────────
#
# `dev-up-mock`   — offline stack: parser_grpc_server + parser_gateway pointed
#                   at the bundled mock_facilitator. Useful when network egress
#                   to facilitator.payai.network isn't available.
# `dev-up-payai`  — real-facilitator stack: parser_grpc_server + parser_gateway
#                   pointed at https://facilitator.payai.network with
#                   X402_NETWORK=solana-devnet. Requires public egress.
#                   Set TVC_DEMO_PINNED_PUBKEY_HEX before running this
#                   target; otherwise the gateway fail-closes.
# Both compose files consume the locally-built stagex images. Build them
# first with `make non-oci-docker-images`.
.PHONY: dev-up-mock dev-up-payai dev-down dev-logs

dev-up-mock: non-oci-docker-images
	docker compose -f compose.mock.yml up -d

dev-up-payai: non-oci-docker-images
	@if [ -z "$$TVC_DEMO_PINNED_PUBKEY_HEX" ]; then \
		echo "ERROR: TVC_DEMO_PINNED_PUBKEY_HEX must be set (the gateway fail-closes without it for X402_PROFILE=payai)."; \
		exit 1; \
	fi
	docker compose -f compose.payai.yml up -d

dev-down:
	-docker compose -f compose.mock.yml down --remove-orphans 2>/dev/null || true
	-docker compose -f compose.payai.yml down --remove-orphans 2>/dev/null || true

dev-logs:
	@if docker compose -f compose.payai.yml ps -q 2>/dev/null | grep -q .; then \
		docker compose -f compose.payai.yml logs -f --tail=100; \
	else \
		docker compose -f compose.mock.yml logs -f --tail=100; \
	fi

define build_context
$$( \
	mkdir -p out; \
	self=$(1); \
	for each in $$(find out/ -maxdepth 2 -name index.json); do \
    	package=$$(basename $$(dirname $${each})); \
    	if [ "$${package}" = "$${self}" ]; then continue; fi; \
    	printf -- ' --build-context %s=oci-layout://./out/%s' "$${package}" "$${package}"; \
	done; \
)
endef

,:=,
define build
	$(eval NAME := $(1))
	$(eval TYPE := $(if $(2),$(2),dir))
	$(eval REGISTRY := anchorageoss-visualsign-parser)
	$(eval PLATFORM := linux/amd64)
	DOCKER_BUILDKIT=1 \
	SOURCE_DATE_EPOCH=1 \
	BUILDKIT_MULTIPLATFORM=1 \
	docker build \
		--build-arg VERSION=$(VERSION) \
		--tag $(REGISTRY)/$(NAME) \
		--progress=plain \
		--platform=$(PLATFORM) \
		--label "org.opencontainers.image.source=https://github.com/anchorageoss/visualsign-parser" \
		$(if $(filter common,$(NAME)),,$(call build_context,$(1))) \
		$(if $(filter 1,$(NOCACHE)),--no-cache) \
		--output "\
			type=oci,\
			$(if $(filter dir,$(TYPE)),tar=false$(,)) \
			rewrite-timestamp=true,\
			force-compression=true,\
			name=$(NAME),\
			$(if $(filter tar,$(TYPE)),dest=$@") \
			$(if $(filter dir,$(TYPE)),dest=out/$(NAME)") \
		-f images/$(NAME)/Containerfile \
		.
endef
