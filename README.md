# Multi-user Chatbot

A local LLM with a web interface, allowing multiple users to chat together.

## Building and Running

Note that a minimum of 8 GB of RAM and a decent CPU is required to run the default model.

### Method 1: Cargo, Bun, llama.cpp

#### Requirements
 - [Bun](https://bun.com/)
 - [Cargo](https://doc.rust-lang.org/cargo/)
 - [llama.cpp](https://github.com/ggml-org/llama.cpp)
This application was tested with Bun 1.4.3 and Cargo 1.95.

1. Build the Svelte frontend.
```bash
cd frontend
bun install
bun run build
```

2. Create the required configuration file.
```bash
cp config.example.toml config.toml
```

3. Build the Rust backend.
```bash
cd ..
cargo build --release
```

4. Start the llama-server on port 8080.
```bash
llama-server -hf <model> --port 8080
```
For example,
```bash
llama-server -hf Qwen/Qwen3-8B-GGUF:Q4_K_M --port 8080 -c 4096 -np 1 --reasoning off
```
Wait for the model to load and the server to start.
A `run-model.sh` script is provided for convenience that will start the server with 4 threads.

5. Run the Rust backend. Note that it will also serve the frontend.
```bash
cargo run --release
```

6. Access the application on [http://localhost:3000](http://localhost:3000).

Note that the server `chat.db` will be created in the project root.

### Method 2: Docker

Requires Docker with the Compose plugin.

1. Create the required configuration file.
```bash
cp config.example.toml config.toml
```

2. Alternatively, you can run the application with [Docker Compose](https://docs.docker.com/compose/).
```bash
docker compose up -d
docker compose logs -f
```
Note that the default model is quite large, around 5 GB.

3. Access the application on [http://localhost:3000](http://localhost:3000).

## Project Structure

```
./
├── data/                  # English vocabulary for username generation
├── frontend/                # Svelte frontend
│   ├── src/                 # Svelte source
│   └── ...                  # Bun config files
├── migrations/            # Database migrations
├── src/                   # Rust backend source
│   ├── chat/                # Message and context handling
│   ├── db/                  # Persistent storage management
│   ├── llm/                 # llama.cpp interfacing
│   ├── names/               # Username generation
│   ├── scheduler/           # Message request handling
│   ├── config.rs            # Config parsing and handling
│   └── main.rs              # Main entry point
├── compose.yaml           # Docker Compose configuration
├── config.example.toml    # Example config file.
├── Dockerfile             # Docker configuration
├── README.md              # This file
└── run-model.sh*          # Run Qwen3 8B llama-server with 4 threads
```
