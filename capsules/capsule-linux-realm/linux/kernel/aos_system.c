// SPDX-License-Identifier: GPL-2.0-only
/*
 * Read-only block device backed by an immutable AOS compute-worker asset.
 *
 * Every bio is split into bounded page segments and synchronously suspended at
 * the private AOS SBI boundary. The signed Linux vCPU worker copies the exact
 * requested asset range into admitted guest RAM, then resumes this hart. There
 * is no host path, DMA queue, writable command, or guest-selected asset index.
 */

#define pr_fmt(fmt) KBUILD_MODNAME ": " fmt

#include <linux/bio.h>
#include <linux/blkdev.h>
#include <linux/init.h>
#include <linux/module.h>
#include <linux/sizes.h>
#include <asm/sbi.h>

#define SBI_EXT_AOS_9P 0x08414f53
#define SBI_FID_AOS_9P_EXCHANGE 0
#define AOS_SYSTEM_CHANNEL 3
#define AOS_SYSTEM_SECTOR_SHIFT 9
#define AOS_SYSTEM_MAX_BYTES SZ_2G
#define AOS_SYSTEM_MAX_READ_BYTES SZ_64K

static unsigned long long aos_system_bytes;
static int aos_system_major;
static struct gendisk *aos_system_disk;

static int __init aos_system_size(char *value)
{
	unsigned long long bytes;

	if (kstrtoull(value, 0, &bytes) || bytes < SZ_4K ||
	    bytes > AOS_SYSTEM_MAX_BYTES || bytes & ((1 << AOS_SYSTEM_SECTOR_SHIFT) - 1))
		return 0;
	aos_system_bytes = bytes;
	return 1;
}
__setup("aos.system_bytes=", aos_system_size);

static bool aos_system_read_segment(struct bio_vec *segment, sector_t sector)
{
	u64 asset_offset = (u64)sector << AOS_SYSTEM_SECTOR_SHIFT;
	phys_addr_t destination = page_to_phys(segment->bv_page) + segment->bv_offset;
	struct sbiret result;

	if (segment->bv_len < (1 << AOS_SYSTEM_SECTOR_SHIFT) ||
	    segment->bv_len > AOS_SYSTEM_MAX_READ_BYTES ||
	    segment->bv_len & ((1 << AOS_SYSTEM_SECTOR_SHIFT) - 1) ||
	    asset_offset > aos_system_bytes ||
	    segment->bv_len > aos_system_bytes - asset_offset)
		return false;

	/*
	 * The eight-byte request is copied before the Realm suspends. The response
	 * range is the bio page itself, so no kernel or guest pointer crosses the
	 * worker boundary.
	 */
	mb();
	result = sbi_ecall(SBI_EXT_AOS_9P, SBI_FID_AOS_9P_EXCHANGE,
			   virt_to_phys(&asset_offset), sizeof(asset_offset),
			   destination, segment->bv_len,
			   AOS_SYSTEM_CHANNEL, 0);
	mb();
	return result.error == 0 && result.value == segment->bv_len;
}

static void aos_system_submit_bio(struct bio *bio)
{
	struct bvec_iter iter;
	struct bio_vec segment;
	sector_t sector = bio->bi_iter.bi_sector;

	if (bio_op(bio) != REQ_OP_READ) {
		bio_io_error(bio);
		return;
	}

	bio_for_each_segment(segment, bio, iter) {
		if (!aos_system_read_segment(&segment, sector)) {
			bio_io_error(bio);
			return;
		}
		sector += segment.bv_len >> AOS_SYSTEM_SECTOR_SHIFT;
	}
	bio_endio(bio);
}

static const struct block_device_operations aos_system_fops = {
	.owner = THIS_MODULE,
	.submit_bio = aos_system_submit_bio,
};

static int __init aos_system_init(void)
{
	struct queue_limits limits = {
		.logical_block_size = 1 << AOS_SYSTEM_SECTOR_SHIFT,
		.physical_block_size = SZ_4K,
		.max_hw_sectors = AOS_SYSTEM_MAX_READ_BYTES >> AOS_SYSTEM_SECTOR_SHIFT,
		.max_segment_size = PAGE_SIZE,
		.features = BLK_FEAT_SYNCHRONOUS,
	};
	int error;

	if (!aos_system_bytes)
		return -ENODEV;

	aos_system_major = register_blkdev(0, "aos-system");
	if (aos_system_major < 0)
		return aos_system_major;

	aos_system_disk = blk_alloc_disk(&limits, NUMA_NO_NODE);
	if (IS_ERR(aos_system_disk)) {
		error = PTR_ERR(aos_system_disk);
		goto unregister;
	}
	aos_system_disk->major = aos_system_major;
	aos_system_disk->first_minor = 0;
	aos_system_disk->minors = 1;
	aos_system_disk->fops = &aos_system_fops;
	strscpy(aos_system_disk->disk_name, "aos-system", DISK_NAME_LEN);
	set_capacity(aos_system_disk, aos_system_bytes >> AOS_SYSTEM_SECTOR_SHIFT);
	set_disk_ro(aos_system_disk, true);
	error = add_disk(aos_system_disk);
	if (error)
		goto put_disk;

	pr_info("admitted %llu-byte immutable system image\n", aos_system_bytes);
	return 0;

put_disk:
	put_disk(aos_system_disk);
unregister:
	unregister_blkdev(aos_system_major, "aos-system");
	return error;
}
device_initcall(aos_system_init);

MODULE_AUTHOR("Unicity Labs");
MODULE_DESCRIPTION("AOS Realm immutable system block device");
MODULE_LICENSE("GPL");
