from pydantic import BaseModel, EmailStr, Field


class UserProfileResponse(BaseModel):
    id: str
    username: str
    email: str
    role: str
    avatar_url: str | None


class UpdateProfileRequest(BaseModel):
    username: str = Field(min_length=3, max_length=64)
    email: EmailStr


class ChangePasswordRequest(BaseModel):
    current_password: str = Field(min_length=1)
    new_password: str = Field(min_length=8, max_length=128)


class AvatarUploadResponse(BaseModel):
    avatar_url: str
