from contextlib import asynccontextmanager
import shlex

from fastapi import FastAPI, Request
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import JSONResponse
import uvicorn

from app.api.v1.router import api_router
from app.api.v1.relays import install_relay_openapi
from app.core.config import settings
from app.core.response_security import SensitiveResponseCacheMiddleware
from app.db.init_db import seed_initial_data
from app.db.migrate_add_relay_control import migrate as migrate_relay_control
from app.db.migrate_add_relay_access import migrate as migrate_relay_access
from app.db.migrate_add_relay_redundancy import migrate as migrate_relay_redundancy
from app.db.migrate_add_browser_controllers import migrate as migrate_browser_controllers
from app.db.session import AsyncSessionLocal, Base, engine
from app.middleware.relay_node_boundary import RelayNodeBoundaryMiddleware
from app.services.realtime_manager import RealtimeSidecarManager
import app.models  # noqa: F401


@asynccontextmanager
async def lifespan(_: FastAPI):
    async with engine.begin() as conn:
        await migrate_relay_control(conn)
        # legacy/dev bootstrap only. Relay tables are created and verified by the
        # explicit versioned migration above, never by metadata.create_all.
        await conn.run_sync(Base.metadata.create_all)
        await migrate_relay_access(conn, serial_pepper=settings.device_serial_pepper)
        await migrate_relay_redundancy(conn)
        await migrate_browser_controllers(conn)
    async with AsyncSessionLocal() as db:
        # Administrator creation is disabled unless every opt-in bootstrap
        # setting is explicitly supplied; no built-in credential exists.
        await seed_initial_data(db, configuration=settings)
    yield


app = FastAPI(title="Rdesk-Server", version="0.1.0", lifespan=lifespan)
app.state.realtime_manager = RealtimeSidecarManager(
    health_url=settings.realtime_server_health_url,
    command=[
        settings.realtime_server_command,
        *shlex.split(settings.realtime_server_args),
    ],
    workdir=settings.realtime_server_workdir,
)


class _RequestBodyTooLarge(Exception):
    pass


@app.middleware("http")
async def reject_oversized_requests(request: Request, call_next):
    """Reject oversized bodies before FastAPI parses or stores them."""

    limit = 8 * 1024 * 1024
    if request.url.path.startswith("/api/v1/auth/"):
        limit = 64 * 1024
    elif request.url.path.endswith("/avatar"):
        limit = 6 * 1024 * 1024
    declared = request.headers.get("content-length")
    if declared is not None:
        try:
            length = int(declared)
        except ValueError:
            length = limit + 1
        if length < 0 or length > limit:
            return JSONResponse(
                {"detail": "request body is too large"}, status_code=413
            )

    # Content-Length is optional for chunked requests. Wrap the ASGI receive
    # callable as well so a client cannot bypass the limit by streaming an
    # unbounded body.
    received = 0
    receive = request._receive

    async def limited_receive():
        nonlocal received
        message = await receive()
        if message.get("type") == "http.request":
            received += len(message.get("body", b""))
            if received > limit:
                raise _RequestBodyTooLarge
        return message

    request._receive = limited_receive
    try:
        return await call_next(request)
    except _RequestBodyTooLarge:
        return JSONResponse({"detail": "request body is too large"}, status_code=413)

app.add_middleware(
    CORSMiddleware,
    allow_origins=[item.strip() for item in settings.cors_origins.split(",") if item.strip()],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)
# Keep this as the outermost application middleware.  Uvicorn proxy rewriting is
# disabled below; the private mTLS proxy must strip forwarding/client auth headers.
app.add_middleware(
    RelayNodeBoundaryMiddleware, trusted_proxy=settings.trusted_mtls_proxy
)
# The cache policy wraps the final response so exception handlers, validation
# routes, and relay-boundary rejections cannot accidentally become cacheable.
app.add_middleware(SensitiveResponseCacheMiddleware)

app.include_router(api_router)
install_relay_openapi(app)


@app.get("/healthz")
async def healthz():
    return {"status": "ok"}


if __name__ == "__main__":
    uvicorn.run(
        "app.main:app",
        host=settings.server_host,
        port=settings.server_port,
        reload=settings.development_reload,
        workers=1,
        proxy_headers=False,
        reload_dirs=["app"] if settings.development_reload else None,
    )
