#!/bin/bash

# Build Docker images for Prediction Market services
# Usage: ./build-docker.sh [service_name]
# If no service name is provided, builds all services

set -e

SERVICES=(
    "api"
    "asset"
    "match_engine"
    "store"
    "processor"
    "depth"
    "event"
    "onchain_msg"
    "websocket_depth"
    "websocket_user"
)

build_service() {
    local service=$1
    echo "=========================================="
    echo "Building $service..."
    echo "=========================================="
    docker-compose build "$service"
    echo "✅ $service built successfully"
    echo ""
}

if [ $# -eq 0 ]; then
    # Build all services
    echo "Building all services..."
    echo ""
    for service in "${SERVICES[@]}"; do
        build_service "$service"
    done
    echo "=========================================="
    echo "✅ All services built successfully!"
    echo "=========================================="
    echo ""
    echo "To start all services:"
    echo "  docker-compose up -d"
    echo ""
    echo "To view logs:"
    echo "  docker-compose logs -f"
else
    # Build specific service
    service=$1
    if [[ " ${SERVICES[@]} " =~ " ${service} " ]]; then
        build_service "$service"
        echo "To start this service:"
        echo "  docker-compose up -d $service"
        echo ""
        echo "To view logs:"
        echo "  docker-compose logs -f $service"
    else
        echo "❌ Error: Unknown service '$service'"
        echo ""
        echo "Available services:"
        for s in "${SERVICES[@]}"; do
            echo "  - $s"
        done
        exit 1
    fi
fi
