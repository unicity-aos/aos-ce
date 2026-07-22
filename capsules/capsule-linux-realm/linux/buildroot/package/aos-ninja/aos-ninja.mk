################################################################################
#
# aos-ninja
#
################################################################################

AOS_NINJA_VERSION = 1.13.2
AOS_NINJA_SITE = $(call github,ninja-build,ninja,v$(AOS_NINJA_VERSION))
AOS_NINJA_LICENSE = Apache-2.0
AOS_NINJA_LICENSE_FILES = COPYING
AOS_NINJA_CONF_OPTS = -DNINJA_BUILD_TESTS=OFF

$(eval $(cmake-package))
