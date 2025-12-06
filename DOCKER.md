# Docker Deployment Guide

This guide explains how to deploy the Prediction Market system using Docker.

## Prerequisites

- Docker 20.10+
- Docker Compose 2.0+
- At least 4GB RAM
- At least 10GB disk space

## Quick Start

### 1. Build All Services

```bash
docker-compose build
```

This will build all microservices. The first build may take 10-20 minutes depending on your machine.

### 2. Start All Services

```bash
docker-compose up -d
```

This will start all services in detached mode:
- PostgreSQL (port 5432)
- Redis (port 6379)
- API (port 5002)
- WebSocket User (port 5003)
- WebSocket Depth (port 5004)
- Asset gRPC (port 50051)
- And all other background services

### 3. Check Service Status

```bash
docker-compose ps
```

### 4. View Logs

View all logs:
```bash
docker-compose logs -f
```

View specific service logs:
```bash
docker-compose logs -f api
docker-compose logs -f match_engine
docker-compose logs -f processor
```

### 5. Stop All Services

```bash
docker-compose down
```

To also remove volumes (database data):
```bash
docker-compose down -v
```

## Service Architecture

```
┌─────────────┐
│   Client    │
└──────┬──────┘
       │
       ↓
┌─────────────────────────────────────────┐
│  Frontend Services                       │
│  - API (REST)                    :5002  │
│  - WebSocket User                :5003  │
│  - WebSocket Depth               :5004  │
└──────────────┬──────────────────────────┘
               │
               ↓
┌─────────────────────────────────────────┐
│  Core Services                           │
│  - Asset (gRPC)              :50051     │
│  - Match Engine                          │
│  - Store                                 │
│  - Event                                 │
│  - Processor                             │
│  - Depth                                 │
│  - Onchain Message                       │
└──────────────┬──────────────────────────┘
               │
               ↓
┌─────────────────────────────────────────┐
│  Infrastructure                          │
│  - PostgreSQL                    :5432  │
│  - Redis                         :6379  │
└─────────────────────────────────────────┘
```

## Building Individual Services

If you want to build a specific service:

```bash
docker-compose build api
docker-compose build match_engine
docker-compose build processor
```

## Running Individual Services

```bash
docker-compose up -d api
docker-compose up -d match_engine
```

## Environment Variables

The services use environment variables from:
- `deploy/common.env` - Shared configuration (Redis, PostgreSQL)
- `deploy/<service>/*.toml` - Service-specific configuration

To customize:
1. Edit `deploy/common.env` for database and Redis connection strings
2. Edit service-specific `.toml` files in `deploy/<service>/` directories

## Health Checks

PostgreSQL and Redis have health checks configured. Other services will wait for these to be healthy before starting.

Check health status:
```bash
docker-compose ps
```

## Volumes

Persistent data is stored in Docker volumes:
- `postgres_data` - PostgreSQL database
- `redis_data` - Redis persistence
- `./logs` - Service logs (mounted from host)
- `./data/store` - Store service snapshots (mounted from host)
- `./data/match_engine` - Match engine snapshots (mounted from host)

## Networking

All services communicate through the `prediction_network` bridge network. This allows services to reach each other using their container names (e.g., `postgres`, `redis`, `asset`).

## Troubleshooting

### Service fails to start

Check logs:
```bash
docker-compose logs <service-name>
```

### Database connection issues

Ensure PostgreSQL is healthy:
```bash
docker-compose ps postgres
```

Reset database:
```bash
docker-compose down -v
docker-compose up -d postgres
```

### Redis connection issues

Check Redis:
```bash
docker-compose logs redis
docker exec -it prediction_redis redis-cli ping
```

### Out of memory

Increase Docker memory limit in Docker Desktop settings or add to docker-compose.yml:
```yaml
services:
  api:
    mem_limit: 512m
```

## Production Deployment

For production, consider:

1. **Use specific image tags** instead of `latest`
2. **Configure resource limits**:
   ```yaml
   services:
     api:
       deploy:
         resources:
           limits:
             cpus: '1'
             memory: 512M
   ```

3. **Use secrets** for sensitive data instead of environment files
4. **Set up monitoring** (Prometheus, Grafana)
5. **Configure log rotation**
6. **Use external PostgreSQL and Redis** for better reliability
7. **Set up backups** for PostgreSQL and Redis data
8. **Use reverse proxy** (Nginx, Traefik) for HTTPS and load balancing

## Development Workflow

1. Make code changes
2. Rebuild the affected service:
   ```bash
   docker-compose build <service>
   ```
3. Restart the service:
   ```bash
   docker-compose up -d <service>
   ```
4. View logs:
   ```bash
   docker-compose logs -f <service>
   ```

## Cleanup

Remove all containers, networks, and images:
```bash
docker-compose down --rmi all -v
```

Remove only containers and networks (keep images):
```bash
docker-compose down
```

## Support

For issues or questions:
- Check service logs: `docker-compose logs <service>`
- Verify service health: `docker-compose ps`
- Review configuration files in `deploy/` directory
