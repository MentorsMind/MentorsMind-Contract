@echo off
REM MentorMinds Local Development Environment Setup Script (Windows)
REM This script sets up a complete local Soroban development environment

setlocal enabledelayedexpansion

echo [INFO] Setting up MentorMinds local development environment...

REM Check if Docker is running
docker info >nul 2>&1
if errorlevel 1 (
    echo [ERROR] Docker is not running. Please start Docker Desktop first.
    pause
    exit /b 1
)
echo [SUCCESS] Docker is running

REM Stop existing container if running
echo [INFO] Stopping existing Stellar container...
docker stop mentorminds-stellar >nul 2>&1
docker rm mentorminds-stellar >nul 2>&1

REM Start new container
echo [INFO] Starting Stellar quickstart container...
docker-compose up -d

echo [INFO] Waiting for Stellar services to be ready...
timeout /t 10 /nobreak >nul

REM Wait for Horizon to be ready
set max_attempts=30
set attempt=1

:wait_horizon
curl -s http://localhost:8000 >nul 2>&1
if errorlevel 1 (
    if %attempt% geq %max_attempts% (
        echo [ERROR] Stellar Horizon failed to start after %max_attempts% attempts
        pause
        exit /b 1
    )
    echo [INFO] Waiting for Horizon... (attempt %attempt%/%max_attempts%)
    timeout /t 2 /nobreak >nul
    set /a attempt+=1
    goto wait_horizon
)
echo [SUCCESS] Stellar Horizon is ready

REM Wait for Soroban RPC to be ready
set attempt=1

:wait_soroban
curl -s http://localhost:8003 >nul 2>&1
if errorlevel 1 (
    if %attempt% geq %max_attempts% (
        echo [ERROR] Soroban RPC failed to start after %max_attempts% attempts
        pause
        exit /b 1
    )
    echo [INFO] Waiting for Soroban RPC... (attempt %attempt%/%max_attempts%)
    timeout /t 2 /nobreak >nul
    set /a attempt+=1
    goto wait_soroban
)
echo [SUCCESS] Soroban RPC is ready

REM Configure Soroban CLI
echo [INFO] Configuring Soroban CLI for local network...
soroban config network remove standalone >nul 2>&1
soroban config network add standalone --rpc-url http://localhost:8003 --network-passphrase "Standalone Network ; February 2017"
echo [SUCCESS] Soroban CLI configured for local network

REM Create deployed directory
if not exist deployed mkdir deployed

REM Create accounts
echo [INFO] Creating and funding test accounts...

REM Initialize accounts file
echo { > deployed\accounts.json
echo   "network": "standalone", >> deployed\accounts.json
echo   "horizon_url": "http://localhost:8000", >> deployed\accounts.json
echo   "rpc_url": "http://localhost:8003", >> deployed\accounts.json
echo   "friendbot_url": "http://localhost:8002", >> deployed\accounts.json
echo   "accounts": { >> deployed\accounts.json

REM Create admin account
echo [INFO] Creating account: admin
soroban config identity remove local_admin >nul 2>&1
soroban config identity generate local_admin
for /f "delims=" %%i in ('soroban config identity address local_admin') do set admin_address=%%i
echo [INFO] Funding account admin (!admin_address!)...
curl -X POST "http://localhost:8002?addr=!admin_address!" >nul 2>&1
timeout /t 1 /nobreak >nul
echo "admin": { >> deployed\accounts.json
echo   "address": "!admin_address!", >> deployed\accounts.json
echo   "identity": "local_admin", >> deployed\accounts.json
echo   "role": "Admin account for platform management" >> deployed\accounts.json
echo }, >> deployed\accounts.json

REM Create mentor1 account
echo [INFO] Creating account: mentor1
soroban config identity remove local_mentor1 >nul 2>&1
soroban config identity generate local_mentor1
for /f "delims=" %%i in ('soroban config identity address local_mentor1') do set mentor1_address=%%i
echo [INFO] Funding account mentor1 (!mentor1_address!)...
curl -X POST "http://localhost:8002?addr=!mentor1_address!" >nul 2>&1
timeout /t 1 /nobreak >nul
echo "mentor1": { >> deployed\accounts.json
echo   "address": "!mentor1_address!", >> deployed\accounts.json
echo   "identity": "local_mentor1", >> deployed\accounts.json
echo   "role": "First mentor account" >> deployed\accounts.json
echo }, >> deployed\accounts.json

REM Create mentor2 account
echo [INFO] Creating account: mentor2
soroban config identity remove local_mentor2 >nul 2>&1
soroban config identity generate local_mentor2
for /f "delims=" %%i in ('soroban config identity address local_mentor2') do set mentor2_address=%%i
echo [INFO] Funding account mentor2 (!mentor2_address!)...
curl -X POST "http://localhost:8002?addr=!mentor2_address!" >nul 2>&1
timeout /t 1 /nobreak >nul
echo "mentor2": { >> deployed\accounts.json
echo   "address": "!mentor2_address!", >> deployed\accounts.json
echo   "identity": "local_mentor2", >> deployed\accounts.json
echo   "role": "Second mentor account" >> deployed\accounts.json
echo }, >> deployed\accounts.json

REM Create learner1 account
echo [INFO] Creating account: learner1
soroban config identity remove local_learner1 >nul 2>&1
soroban config identity generate local_learner1
for /f "delims=" %%i in ('soroban config identity address local_learner1') do set learner1_address=%%i
echo [INFO] Funding account learner1 (!learner1_address!)...
curl -X POST "http://localhost:8002?addr=!learner1_address!" >nul 2>&1
timeout /t 1 /nobreak >nul
echo "learner1": { >> deployed\accounts.json
echo   "address": "!learner1_address!", >> deployed\accounts.json
echo   "identity": "local_learner1", >> deployed\accounts.json
echo   "role": "First learner account" >> deployed\accounts.json
echo }, >> deployed\accounts.json

REM Create learner2 account
echo [INFO] Creating account: learner2
soroban config identity remove local_learner2 >nul 2>&1
soroban config identity generate local_learner2
for /f "delims=" %%i in ('soroban config identity address local_learner2') do set learner2_address=%%i
echo [INFO] Funding account learner2 (!learner2_address!)...
curl -X POST "http://localhost:8002?addr=!learner2_address!" >nul 2>&1
timeout /t 1 /nobreak >nul
echo "learner2": { >> deployed\accounts.json
echo   "address": "!learner2_address!", >> deployed\accounts.json
echo   "identity": "local_learner2", >> deployed\accounts.json
echo   "role": "Second learner account" >> deployed\accounts.json
echo } >> deployed\accounts.json
echo   } >> deployed\accounts.json
echo } >> deployed\accounts.json

echo [SUCCESS] All accounts created and funded

REM Build contracts
echo [INFO] Building all contracts...

echo [INFO] Building escrow contract...
cd escrow
cargo build --target wasm32-unknown-unknown --release
soroban contract optimize --wasm target\wasm32-unknown-unknown\release\mentorminds_escrow.wasm
cd ..

echo [INFO] Building verification contract...
cd contracts\verification
cargo build --target wasm32-unknown-unknown --release
soroban contract optimize --wasm target\wasm32-unknown-unknown\release\mentorminds_verification.wasm
cd ..\..

echo [INFO] Building oracle contract...
cd contracts\oracle
cargo build --target wasm32-unknown-unknown --release
soroban contract optimize --wasm target\wasm32-unknown-unknown\release\mentorminds_oracle.wasm
cd ..\..

echo [INFO] Building timelock contract...
cd contracts\timelock
cargo build --target wasm32-unknown-unknown --release
soroban contract optimize --wasm target\wasm32-unknown-unknown\release\mentorminds_timelock.wasm
cd ..\..

echo [SUCCESS] All contracts built successfully

REM Deploy contracts
echo [INFO] Deploying contracts to local network...

REM Initialize contracts file
echo { > deployed\local.json
echo   "network": "standalone", >> deployed\local.json
echo   "deployed_at": "%date:~0,4%-%date:~5,2%-%date:~8,2%T%time:~0,2%:%time:~3,2%:%time:~6,2%Z", >> deployed\local.json
echo   "contracts": { >> deployed\local.json

REM Deploy escrow contract
echo [INFO] Deploying escrow contract...
for /f "delims=" %%i in ('soroban contract deploy --wasm escrow\target\wasm32-unknown-unknown\release\mentorminds_escrow.wasm --source local_admin --network standalone') do set escrow_id=%%i
echo "escrow": { >> deployed\local.json
echo   "contract_id": "!escrow_id!", >> deployed\local.json
echo   "wasm_path": "escrow/target/wasm32-unknown-unknown/release/mentorminds_escrow.wasm" >> deployed\local.json
echo }, >> deployed\local.json

REM Initialize escrow contract
echo [INFO] Initializing escrow contract...
soroban contract invoke --id !escrow_id! --source local_admin --network standalone -- initialize --admin !admin_address! --platform_fee 5

REM Deploy verification contract
echo [INFO] Deploying verification contract...
for /f "delims=" %%i in ('soroban contract deploy --wasm contracts\verification\target\wasm32-unknown-unknown\release\mentorminds_verification.wasm --source local_admin --network standalone') do set verification_id=%%i
echo "verification": { >> deployed\local.json
echo   "contract_id": "!verification_id!", >> deployed\local.json
echo   "wasm_path": "contracts/verification/target/wasm32-unknown-unknown/release/mentorminds_verification.wasm" >> deployed\local.json
echo }, >> deployed\local.json

REM Initialize verification contract
echo [INFO] Initializing verification contract...
soroban contract invoke --id !verification_id! --source local_admin --network standalone -- initialize --admin !admin_address!

REM Deploy oracle contract
echo [INFO] Deploying oracle contract...
for /f "delims=" %%i in ('soroban contract deploy --wasm contracts\oracle\target\wasm32-unknown-unknown\release\mentorminds_oracle.wasm --source local_admin --network standalone') do set oracle_id=%%i
echo "oracle": { >> deployed\local.json
echo   "contract_id": "!oracle_id!", >> deployed\local.json
echo   "wasm_path": "contracts/oracle/target/wasm32-unknown-unknown/release/mentorminds_oracle.wasm" >> deployed\local.json
echo }, >> deployed\local.json

REM Initialize oracle contract
echo [INFO] Initializing oracle contract...
soroban contract invoke --id !oracle_id! --source local_admin --network standalone -- initialize --admin !admin_address!

REM Deploy timelock contract
echo [INFO] Deploying timelock contract...
for /f "delims=" %%i in ('soroban contract deploy --wasm contracts\timelock\target\wasm32-unknown-unknown\release\mentorminds_timelock.wasm --source local_admin --network standalone') do set timelock_id=%%i
echo "timelock": { >> deployed\local.json
echo   "contract_id": "!timelock_id!", >> deployed\local.json
echo   "wasm_path": "contracts/timelock/target/wasm32-unknown-unknown/release/mentorminds_timelock.wasm" >> deployed\local.json
echo } >> deployed\local.json

REM Initialize timelock contract
echo [INFO] Initializing timelock contract...
soroban contract invoke --id !timelock_id! --source local_admin --network standalone -- initialize --admin !admin_address! --min_delay 3600

echo   } >> deployed\local.json
echo } >> deployed\local.json

echo [SUCCESS] All contracts deployed successfully

echo.
echo [SUCCESS] Local development environment setup complete!
echo.
echo [INFO] Available services:
echo   - Horizon API: http://localhost:8000
echo   - Stellar RPC: http://localhost:8001
echo   - Soroban RPC: http://localhost:8003
echo   - Friendbot: http://localhost:8002
echo.
echo [INFO] Configuration files:
echo   - Accounts: deployed\accounts.json
echo   - Contracts: deployed\local.json
echo.
echo [INFO] Next steps:
echo   1. Run 'npm run local:seed' to create sample data
echo   2. Start backend development with 'npm run dev'
echo   3. Use 'npm run local:stop' to stop the environment
echo.

pause
