out/parser_host/index.json: \
	$(shell git ls-files images/parser_host src)
	$(call build,parser_host)

out/parser_app/index.json: \
	$(shell git ls-files images/parser_app src)
	$(call build,parser_app)

.PHONY: non-oci-docker-images
non-oci-docker-images:
	docker buildx build --load --tag anchorageoss-visualsign-parser/parser_app -f images/parser_app/Containerfile .
	docker buildx build --load --tag anchorageoss-visualsign-parser/parser_host -f images/parser_host/Containerfile .

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

#Import environment variable into a MAKE variable
GITHUB_TOKEN ?= $(shell echo $$GITHUB_TOKEN)

DOCKER_BUILD_ARGS = --build-arg VERSION=$(VERSION)
# --- Secret Handling ---
ifneq ($(GITHUB_TOKEN),)
# Generate a temporary file name for the secret
# Using `mktemp` for secure temporary file creation
# We use `shell` to execute the command and capture its output
SECRET_FILE := $(shell mktemp -u --tmpdir docker-secret-XXXXXXXXXX)

# Command to write the secret to the temporary file
WRITE_SECRET_CMD := @echo "$(GITHUB_TOKEN)" > $(SECRET_FILE)

# Command to remove the temporary secret file
REMOVE_SECRET_CMD := @rm -f $(SECRET_FILE)

# Docker build arguments for secrets
# This combines the ID and the path to the temporary file
DOCKER_BUILD_SECRET_ARG := --secret id=github_token,src=$(SECRET_FILE)
else 
WRITE_SECRET_CMD := @echo "No GITHUB_TOKEN provided, skipping secret handling."
REMOVE_SECRET_CMD := @true
endif

,:=,
define build
	$(eval NAME := $(1))
	$(eval TYPE := $(if $(2),$(2),dir))
	$(eval REGISTRY := anchorageoss-visualsign-parser)
	$(eval PLATFORM := linux/amd64)
	$(WRITE_SECRET_CMD) && \
	DOCKER_BUILDKIT=1 \
	SOURCE_DATE_EPOCH=1 \
	BUILDKIT_MULTIPLATFORM=1 \
	docker build \
		$(DOCKER_BUILD_ARGS) \
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
		. && \
	$(REMOVE_SECRET_CMD)
endef
