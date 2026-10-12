#!/bin/bash

llama-server -hf Qwen/Qwen3-8B-GGUF:Q4_K_M --port 8080 -c 4096 -np 1 -t 4 --jinja --reasoning off
