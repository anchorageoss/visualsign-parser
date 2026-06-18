# Docs dev tooling — Mintlify in a container.
#
# `make dev` is the common case: builds the image (if needed) and starts
# the local dev server with this directory bind-mounted, so edits show
# up live.

IMAGE_NAME ?= visualsign-mintlify
PORT       ?= 3000
ENGINE     ?= docker

.PHONY: build-image dev broken-links shell rss

build-image:
	$(ENGINE) build -t $(IMAGE_NAME) -f Containerfile .

dev: build-image
	$(ENGINE) run --rm -it \
		-p $(PORT):3000 \
		-v $(CURDIR):/docs \
		$(IMAGE_NAME)

broken-links: build-image
	$(ENGINE) run --rm \
		-v $(CURDIR):/docs \
		$(IMAGE_NAME) mint broken-links

# Open an interactive shell in the container for ad-hoc CLI work.
shell: build-image
	$(ENGINE) run --rm -it \
		-v $(CURDIR):/docs \
		--entrypoint sh \
		$(IMAGE_NAME)

# Fetch the auto-generated RSS feeds from a running dev server (port $(PORT)).
# Run `make dev` in another terminal first.
rss:
	@echo "Changelog feed:" && curl -sf http://localhost:$(PORT)/changelog/rss.xml | head -40
	@echo "Blog feed:"      && curl -sf http://localhost:$(PORT)/blog/rss.xml      | head -40
