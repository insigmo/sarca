up:
	docker compose up -d --force-recreate --remove-orphans

# Local image build (requires a temporary `build: .` on the sarca service)
up-build:
	docker compose up -d --build --force-recreate --remove-orphans

down:
	docker compose down

run_ui:
	cd ui && pnpm run dev || cd -
