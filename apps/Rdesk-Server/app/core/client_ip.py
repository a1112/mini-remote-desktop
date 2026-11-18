from ipaddress import IPv4Network, IPv6Network, ip_address, ip_network

from starlette.requests import Request


class InvalidForwardedAddress(ValueError):
    """A configured proxy supplied invalid client address metadata."""


def proxy_networks(value: str) -> tuple[IPv4Network | IPv6Network, ...]:
    return tuple(ip_network(entry.strip()) for entry in value.split(",") if entry.strip())


def trusted_client_ip(request: Request, trusted_proxies: str) -> str:
    """Use at most one X-Real-IP value supplied by an explicit socket peer."""
    if request.client is None:
        raise ValueError("Missing socket peer")
    peer = ip_address(request.client.host)
    normalized_peer = getattr(peer, "ipv4_mapped", None) or peer
    if any(peer in network or normalized_peer in network for network in proxy_networks(trusted_proxies)):
        forwarded = request.headers.getlist("x-real-ip")
        if forwarded:
            if len(forwarded) != 1 or "%" in forwarded[0]:
                raise InvalidForwardedAddress("Expected a single client IP address")
            try:
                address = ip_address(forwarded[0])
            except ValueError as exc:
                raise InvalidForwardedAddress("Invalid client IP address") from exc
            return str(getattr(address, "ipv4_mapped", None) or address)
    return str(normalized_peer)
