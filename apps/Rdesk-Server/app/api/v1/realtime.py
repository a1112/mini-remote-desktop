from fastapi import APIRouter, Request

from app.services.realtime_manager import RealtimeSidecarManager

router = APIRouter(prefix="/realtime", tags=["realtime"])


def _manager(request: Request) -> RealtimeSidecarManager:
    return request.app.state.realtime_manager


@router.get("/status")
async def realtime_status(request: Request) -> dict:
    status = _manager(request).status()
    return {
        "running": status.running,
        "reachable": status.reachable,
        "status": status.status,
        "pid": status.pid,
    }


@router.post("/start")
async def realtime_start(request: Request) -> dict:
    status = _manager(request).start()
    return {
        "running": status.running,
        "reachable": status.reachable,
        "status": status.status,
        "pid": status.pid,
    }


@router.post("/stop")
async def realtime_stop(request: Request) -> dict:
    status = _manager(request).stop()
    return {
        "running": status.running,
        "reachable": status.reachable,
        "status": status.status,
        "pid": status.pid,
    }


@router.post("/restart")
async def realtime_restart(request: Request) -> dict:
    status = _manager(request).restart()
    return {
        "running": status.running,
        "reachable": status.reachable,
        "status": status.status,
        "pid": status.pid,
    }
